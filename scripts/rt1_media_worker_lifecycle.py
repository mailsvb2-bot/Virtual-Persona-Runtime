"""RT1 worker-side durable allocation and sender shutdown coordinator.

Not a WebRTC implementation. The caller must supply an actual SenderBackend
that does not acknowledge stop until RTP audio and video senders are stopped.
"""
from __future__ import annotations

import os
import sqlite3
import time
from queue import Queue, Empty
import threading
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol


@dataclass(frozen=True)
class StopReceipt:
    operation_id: str
    session_id: str
    generation: int
    audio_stopped: bool
    video_stopped: bool


class AllocatedSessionError(RuntimeError):
    """Provider CREATE failed after returning an authoritative remote sender ID.

    Only the adapter that actually received the provider's allocation identity
    may construct this error. It is not a guessed ID or a STOP receipt.
    """

    def __init__(self, operation_id: str, generation: int, session_id: str):
        if not operation_id or generation < 1 or not session_id:
            raise ValueError("invalid allocated sender identity")
        super().__init__("provider CREATE failed after remote allocation")
        self.operation_id = operation_id
        self.generation = generation
        self.session_id = session_id


class SenderBackend(Protocol):
    def allocate_bounded(
        self,
        operation_id: str,
        generation: int,
        timeout: float,
        cancel_requested: threading.Event,
    ) -> str:
        """Create under the caller deadline and observe owner cancellation.

        Implementations must enforce timeout in their provider transport, not
        by spawning a detached waiter. If a provider has already allocated and
        can return its authoritative late ID, return it for durable STOP.
        """

    def persona_video_ready(self) -> bool:
        """True only when a real Persona renderer is loaded and ready.

        A transport test-pattern video track is not a Persona renderer.
        Missing or failing implementations are treated as not ready.
        """

    def stop(self, operation_id: str, session_id: str, generation: int) -> StopReceipt:
        """Stop BOTH real senders, return a verified receipt after completion."""


class MediaLifecycle:
    """One serialized control plane; uncertain states never admit replacement."""

    def __init__(self, path: str | Path, backend: SenderBackend):
        self._db = sqlite3.connect(str(path), check_same_thread=False, isolation_level=None)
        self._db.execute("PRAGMA journal_mode=WAL")
        self._db.execute("PRAGMA synchronous=FULL")
        self._db.execute("PRAGMA busy_timeout=5000")
        self._db.execute("""CREATE TABLE IF NOT EXISTS media_operations (
            id TEXT PRIMARY KEY, generation INTEGER NOT NULL,
            phase TEXT NOT NULL, session_id TEXT,
            preassigned_session_id TEXT,
            actual_stop_confirmed INTEGER NOT NULL DEFAULT 0,
            stop_verified INTEGER NOT NULL DEFAULT 0,
            CHECK(phase IN ('creating', 'active', 'stopping', 'stopped', 'uncertain'))
        )""")
        # Existing SQLite ledgers predate preassigned identity. Migrate in a
        # write transaction so concurrent worker startups cannot race ALTER.
        self._db.execute("BEGIN IMMEDIATE")
        try:
            columns = {
                row[1] for row in self._db.execute("PRAGMA table_info(media_operations)")
            }
            if "preassigned_session_id" not in columns:
                self._db.execute(
                    "ALTER TABLE media_operations ADD COLUMN preassigned_session_id TEXT"
                )
            if "actual_stop_confirmed" not in columns:
                self._db.execute(
                    "ALTER TABLE media_operations ADD COLUMN actual_stop_confirmed INTEGER NOT NULL DEFAULT 0"
                )
            if "stop_verified" not in columns:
                self._db.execute(
                    "ALTER TABLE media_operations ADD COLUMN stop_verified INTEGER NOT NULL DEFAULT 0"
                )
            self._db.execute("COMMIT")
        except BaseException:
            if self._db.in_transaction:
                self._db.execute("ROLLBACK")
            raise
        self._lock = threading.RLock()
        # An OS shutdown signal permanently fences all later CREATE requests.
        # A live CREATE is quarantined in SQLite before media teardown.
        self._closing = threading.Event()
        # A timed-out CREATE still owns its provider thread until it exits.
        # Bound outstanding allocations instead of spawning one per HTTP PUT.
        self._create_gate = threading.BoundedSemaphore(1)
        # The HTTP deadline can expire before SQLite accepts a CANCEL write.
        # Fence that still-running CREATE locally before it can commit active.
        # This is NOT a substitute for the durable tombstone after a crash.
        # Cancellation is delivered to the provider transport itself. No
        # detached CREATE waiter thread may outlive the HTTP request.
        self._create_cancellations: dict[tuple[str, int], threading.Event] = {}
        # Kept separate from the SQLite RLock so shutdown can signal an in-flight
        # provider call even while a durable writer is blocked.
        self._create_cancellations_lock = threading.Lock()
        # A hung STOP owns one worker thread until the backend returns.
        # Repeated HTTP DELETEs must not accumulate unbounded STOP threads.
        self._stop_gate = threading.BoundedSemaphore(1)
        self._stop_lock_path = str(path) + ".stop.lock"
        self._backend = backend

    def persona_video_ready(self) -> bool:
        """Fail closed when the backend lacks a real Persona renderer."""
        probe = getattr(self._backend, "persona_video_ready", None)
        try:
            return callable(probe) and probe() is True
        except Exception:
            return False

    def persona_identity(self) -> tuple[str, int] | None:
        """Read the backend's immutable asset binding, not a second registry."""
        identity = getattr(self._backend, "persona_identity", None)
        if not callable(identity):
            return None
        result = identity()
        if result is None:
            return None
        if (not isinstance(result, tuple) or len(result) != 2
                or not isinstance(result[0], str)
                or not result[0] or type(result[1]) is not int
                or result[1] < 1):
            raise RuntimeError("invalid worker Persona binding")
        return result

    def publication_readiness(self) -> dict[str, bool | str | int]:
        """Read-only admission snapshot; never CREATE, CANCEL or claim STOP.

        A real Persona video renderer is mandatory for public publication.
        Missing, false, or failing backend capability probes deny publication.
        The durable ledger independently denies unresolved senders/shutdown;
        CREATE still rechecks that state under BEGIN IMMEDIATE.
        """
        persona_video_ready = self.persona_video_ready()
        identity = self.persona_identity()
        metadata = (
            {"persona_id": identity[0], "persona_version": identity[1]}
            if identity is not None else {}
        )
        if self._closing.is_set():
            return {"ready": False, "persona_video_ready": persona_video_ready, **metadata}
        with self._lock:
            if self._closing.is_set():
                ledger_ready = False
            else:
                ledger_ready = self._db.execute(
                    """SELECT 1 FROM media_operations
                       WHERE phase != 'stopped'
                          OR (session_id IS NOT NULL AND stop_verified=0)
                       LIMIT 1"""
                ).fetchone() is None
        return {
            "ready": ledger_ready and persona_video_ready,
            "persona_video_ready": persona_video_ready,
            **metadata,
        }

    def publication_ready(self) -> bool:
        """Compatibility predicate for callers that need only the final gate."""
        return self.publication_readiness()["ready"]

    def _get(self, operation_id: str):
        return self._db.execute(
            "SELECT generation,phase,session_id FROM media_operations WHERE id=?",
            (operation_id,),
        ).fetchone()

    def _write(self, operation_id: str, generation: int, phase: str, session_id: str | None):
        self._db.execute(
            """INSERT INTO media_operations(id,generation,phase,session_id)
               VALUES(?,?,?,?) ON CONFLICT(id) DO UPDATE SET
               generation=excluded.generation,phase=excluded.phase,
               session_id=excluded.session_id""",
            (operation_id, generation, phase, session_id),
        )

    def _validate(self, receipt: StopReceipt, operation_id: str, session_id: str, generation: int):
        return (
            receipt.operation_id == operation_id
            and receipt.session_id == session_id
            and receipt.generation == generation
            and receipt.audio_stopped is True
            and receipt.video_stopped is True
        )

    def create_bounded(self, operation_id: str, generation: int, timeout: float) -> str:
        """Run one CREATE under a deadline enforced by the provider transport.

        No daemon waiter thread is used. The backend receives the remaining
        deadline and the same cancellation event that owner DELETE/shutdown
        sets. If a provider returns an allocation ID after cancellation, the
        durable ledger records and compensates that exact ID before admission.
        """
        if not 0 < timeout <= 60:
            raise ValueError("invalid CREATE timeout")
        return self.create(operation_id, generation, timeout=timeout)

    def create(
        self, operation_id: str, generation: int, timeout: float = 8.0
    ) -> str:
        """Create synchronously through a backend-enforced bounded transport."""
        if not operation_id or generation < 1:
            raise ValueError("invalid operation identity")
        if not 0 < timeout <= 60:
            raise ValueError("invalid CREATE timeout")
        if self._closing.is_set():
            raise RuntimeError("RT1 worker shutting down; CREATE denied")
        if not self._create_gate.acquire(blocking=False):
            raise RuntimeError("another provider CREATE remains in flight")
        identity = (operation_id, generation)
        cancellation = threading.Event()
        deadline = time.monotonic() + timeout
        try:
            with self._create_cancellations_lock:
                self._create_cancellations[identity] = cancellation
            return self._create(operation_id, generation, deadline, cancellation)
        finally:
            with self._create_cancellations_lock:
                self._create_cancellations.pop(identity, None)
            self._create_gate.release()

    def _create(
        self,
        operation_id: str,
        generation: int,
        deadline: float,
        cancellation: threading.Event,
    ) -> str:
        # Obtain the backend's *authoritative* preassigned identity before
        # allocation. Recording it only after a timeout misses process death
        # during CREATE, especially when CANCEL already committed. This is
        # metadata, not evidence that a remote sender exists or stopped.
        recovery_id = None
        identity = getattr(self._backend, "recovery_session_id", None)
        if callable(identity):
            candidate = identity(operation_id, generation)
            if not isinstance(candidate, str) or not candidate:
                raise RuntimeError("backend supplied invalid preassigned sender identity")
            recovery_id = candidate
        with self._lock:
            if self._closing.is_set() or cancellation.is_set():
                raise RuntimeError("RT1 CREATE cancelled before provider dispatch")
            # The Python lock is per process. BEGIN IMMEDIATE also serializes
            # admission across distinct worker processes sharing this ledger.
            # Commit the durable CREATE intent before any provider allocation.
            self._db.execute("BEGIN IMMEDIATE")
            try:
                previous = self._get(operation_id)
                if previous is not None:
                    if previous[0] == generation and previous[1] == "active":
                        self._db.execute("COMMIT")
                        return previous[2]
                    raise RuntimeError("operation already exists or has been cancelled")
                # At most ONE unresolved remote publisher, across processes.
                if self._db.execute(
                    """SELECT 1 FROM media_operations
                       WHERE phase != 'stopped'
                          OR (session_id IS NOT NULL AND stop_verified=0)
                       LIMIT 1"""
                ).fetchone():
                    raise RuntimeError("worker has unresolved media allocation")
                allocate_bounded = getattr(self._backend, "allocate_bounded", None)
                if not callable(allocate_bounded):
                    raise RuntimeError(
                        "backend lacks transport-enforced bounded CREATE; allocation denied"
                    )
                self._write(operation_id, generation, "creating", None)
                if recovery_id is not None:
                    self._db.execute(
                        "UPDATE media_operations SET preassigned_session_id=? WHERE id=?",
                        (recovery_id, operation_id),
                    )
                self._db.execute("COMMIT")
            except BaseException:
                if self._db.in_transaction:
                    self._db.execute("ROLLBACK")
                raise
        try:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or cancellation.is_set():
                raise TimeoutError("provider CREATE deadline/cancellation reached before dispatch")
            session_id = allocate_bounded(
                operation_id, generation, remaining, cancellation
            )
            if not session_id:
                raise RuntimeError("backend supplied an empty session identifier")
        except BaseException as create_error:
            # Capture owner cancellation before our own compensating cancel_bounded()
            # call sets the same event. Otherwise a normal lost CREATE reply would
            # be misreported as an owner revoke after compensation starts.
            owner_cancelled = cancellation.is_set()
            # Some adapters receive the actual provider-generated session ID
            # before a later CREATE validation/transport step fails. Preserve
            # that *authenticated* ID even when the ordinary CREATE reply was
            # lost. An unrelated or mismatched exception is never trusted.
            allocated_id = (
                create_error.session_id
                if isinstance(create_error, AllocatedSessionError)
                and create_error.operation_id == operation_id
                and create_error.generation == generation
                else None
            )
            cleanup_id = allocated_id or recovery_id
            with self._lock:
                # A different worker may CANCEL between a read and a write.
                # Never overwrite its durable tombstone with an old CREATE result.
                self._db.execute("BEGIN IMMEDIATE")
                try:
                    row = self._get(operation_id)
                    if row and row[0] == generation and row[1] in (
                        "creating", "stopping", "uncertain"
                    ) and row[2] is None:
                        # A lost CREATE reply may still have a known sender ID.
                        # Otherwise retain an unknown-id quarantine, never a receipt.
                        self._write(operation_id, generation, "uncertain", cleanup_id)
                    self._db.execute("COMMIT")
                except BaseException:
                    if self._db.in_transaction:
                        self._db.execute("ROLLBACK")
                    raise
            # Compensation uses the existing bounded STOP owner. A stalled
            # provider STOP cannot hold the CREATE HTTP request open.
            if cleanup_id is not None:
                try:
                    self.cancel_bounded(
                        operation_id, generation,
                        min(max(deadline - time.monotonic(), 0.05), 1.0),
                    )
                except Exception:
                    pass
            if time.monotonic() >= deadline:
                raise RuntimeError(
                    "CREATE deadline exceeded; remote STOP unconfirmed"
                ) from create_error
            if owner_cancelled:
                raise RuntimeError(
                    "provider CREATE cancelled; remote STOP may remain unconfirmed"
                ) from create_error
            raise
        if recovery_id is not None and session_id != recovery_id:
            # Persist the actual ID and retain the preassigned identity as a
            # second possible target; both must be stopped before quarantine clears.
            with self._lock:
                self._db.execute("BEGIN IMMEDIATE")
                try:
                    row = self._get(operation_id)
                    if row is None or row[0] != generation or row[1] not in (
                        "creating", "stopping", "uncertain"
                    ):
                        raise RuntimeError("CREATE identity changed during compensation")
                    self._write(operation_id, generation, "uncertain", session_id)
                    self._db.execute("COMMIT")
                except BaseException:
                    if self._db.in_transaction:
                        self._db.execute("ROLLBACK")
                    raise
            try:
                receipt = self.cancel_bounded(
                    operation_id, generation,
                    min(max(deadline - time.monotonic(), 0.05), 1.0),
                )
                if receipt is None:
                    raise RuntimeError("compensating STOP remains pending")
            except Exception as error:
                raise RuntimeError(
                    "backend CREATE identity mismatch; compensating STOP unconfirmed"
                ) from error
            raise RuntimeError(
                "backend CREATE identity mismatch; both possible sender STOPs confirmed"
            )
        with self._lock:
            # CREATE completion and CANCEL have one SQLite linearization point
            # across processes; a stale read can never overwrite a tombstone.
            self._db.execute("BEGIN IMMEDIATE")
            try:
                row = self._get(operation_id)
                if (
                    row
                    and row[0] == generation
                    and row[1] == "creating"
                    and not cancellation.is_set()
                    and time.monotonic() < deadline
                    and not self._closing.is_set()
                ):
                    self._write(operation_id, generation, "active", session_id)
                    self._db.execute("COMMIT")
                    return session_id
                self._write(operation_id, generation, "stopping", session_id)
                self._db.execute("COMMIT")
            except BaseException:
                if self._db.in_transaction:
                    self._db.execute("ROLLBACK")
                raise
        try:
            self.cancel_bounded(
                operation_id, generation,
                min(max(deadline - time.monotonic(), 0.05), 1.0),
            )
        except Exception:
            # STOP remains durable and retryable; never claim a receipt here.
            pass
        if time.monotonic() >= deadline:
            raise RuntimeError("CREATE deadline exceeded; remote STOP unconfirmed")
        raise RuntimeError("operation revoked during allocation")

    def cancel_bounded(
        self, operation_id: str, generation: int, timeout: float
    ) -> StopReceipt | None:
        """Persist CANCEL before bounded STOP; late receipts remain durable.

        Returning None is HTTP 202 (unconfirmed), never a fabricated sender
        receipt. Only one backend STOP thread can be outstanding per worker.
        A late successful STOP commits to SQLite even after HTTP timed out.
        """
        if not 0 < timeout <= 60:
            raise ValueError("invalid STOP timeout")
        receipt = self.cancel(operation_id, generation, wait_for_stop=False)
        if receipt is not None:
            return receipt
        with self._lock:
            row = self._get(operation_id)
        if (
            row is None or row[0] != generation
            or row[1] not in ("stopping", "uncertain") or not row[2]
        ):
            return None
        if not self._stop_gate.acquire(blocking=False):
            return None
        outcome = Queue(maxsize=1)

        def stop():
            try:
                outcome.put((True, self._finish_stop(operation_id, row[2], generation)))
            except Exception as error:
                outcome.put((False, error))
            finally:
                self._stop_gate.release()

        try:
            threading.Thread(
                target=stop, name="rt1-bounded-stop", daemon=True
            ).start()
        except Exception:
            self._stop_gate.release()
            raise
        try:
            succeeded, result = outcome.get(timeout=timeout)
        except Empty:
            # The in-flight STOP retains its durable target and OS lock.
            # A subsequent DELETE/reconciler may observe the late receipt.
            return None
        if succeeded:
            return result
        raise result

    def cancel(
        self, operation_id: str, generation: int, *, wait_for_stop: bool = True
    ) -> StopReceipt | None:
        if not operation_id or generation < 1:
            raise ValueError("invalid operation identity")
        # Signal before waiting for the SQLite RLock/transaction. This must
        # remain prompt even when another in-process writer is stalled.
        with self._create_cancellations_lock:
            pending_create = self._create_cancellations.get((operation_id, generation))
            if pending_create is not None:
                pending_create.set()
        with self._lock:
            # A CANCEL can arrive before CREATE, including from another worker
            # process. Reserve a durable tombstone atomically so that a late
            # CREATE can never allocate after the owner has revoked authority.
            self._db.execute("BEGIN IMMEDIATE")
            try:
                row = self._get(operation_id)
                if row is None:
                    self._write(operation_id, generation, "stopped", None)
                    self._db.execute("COMMIT")
                    # A tombstone prevents future allocation but is not an
                    # actual remote audio/video STOP receipt.
                    return None
                if row[0] != generation:
                    raise KeyError("unknown or stale operation")
                _, phase, session_id = row
                if phase == "stopped":
                    if session_id is None:
                        self._db.execute("COMMIT")
                        return None
                    verified = self._db.execute(
                        "SELECT stop_verified FROM media_operations WHERE id=?",
                        (operation_id,),
                    ).fetchone()[0]
                    if verified == 1:
                        self._db.execute("COMMIT")
                        return StopReceipt(operation_id, session_id, generation, True, True)
                    # Legacy or interrupted writes may say stopped without
                    # durable dual-sender evidence. Retry real STOP, not a
                    # fabricated audio/video receipt.
                    self._write(operation_id, generation, "uncertain", session_id)
                if phase == "creating":
                    self._write(operation_id, generation, "stopping", None)
                    # CREATE completion owns compensating STOP. An unresolved
                    # allocation must never be acknowledged as stopped.
                    self._db.execute("COMMIT")
                    return None
                if not session_id:
                    self._write(operation_id, generation, "uncertain", None)
                    self._db.execute("COMMIT")
                    return None
                self._write(operation_id, generation, "stopping", session_id)
                self._db.execute("COMMIT")
            except BaseException:
                if self._db.in_transaction:
                    self._db.execute("ROLLBACK")
                raise
        if not wait_for_stop:
            # Reconciler retries STOP; no caller may treat this as confirmed.
            return None
        return self._finish_stop(operation_id, session_id, generation)

    @contextmanager
    def _active_publication_transaction(self, operation_id: str, expected_row: tuple):
        """Serialize the final bounded SDP/answer write with durable CANCEL.

        The caller MUST hold _exclusive_stop first: it excludes the physical
        sender STOP while this short transaction protects the browser write.
        A pre/post status check alone has a revoke-to-send TOCTOU race.
        Never run provider I/O or an unbounded client write inside this gate.
        """
        with self._lock:
            self._db.execute("BEGIN IMMEDIATE")
            try:
                yield self._get(operation_id) == expected_row
                self._db.execute("COMMIT")
            except BaseException:
                if self._db.in_transaction:
                    self._db.execute("ROLLBACK")
                raise

    @contextmanager
    def _exclusive_stop(self):
        """OS-held STOP ownership; a crashed process releases it automatically.

        A non-blocking lock avoids waiting behind a hung provider. No second
        caller may send STOP or overwrite its result while the first is alive.
        Reconciliation may retry after the owning process exits.
        """
        fd = os.open(self._stop_lock_path, os.O_CREAT | os.O_RDWR, 0o600)
        locked = False
        try:
            try:
                if os.name == "nt":
                    import msvcrt

                    if os.fstat(fd).st_size == 0:
                        os.write(fd, b"\0")
                    os.lseek(fd, 0, os.SEEK_SET)
                    msvcrt.locking(fd, msvcrt.LK_NBLCK, 1)
                else:
                    import fcntl

                    fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except OSError as error:
                raise RuntimeError(
                    "remote STOP already in progress; shutdown not confirmed"
                ) from error
            locked = True
            yield
        finally:
            if locked:
                if os.name == "nt":
                    os.lseek(fd, 0, os.SEEK_SET)
                    msvcrt.locking(fd, msvcrt.LK_UNLCK, 1)
                else:
                    fcntl.flock(fd, fcntl.LOCK_UN)
            os.close(fd)

    def _finish_stop(self, operation_id: str, session_id: str, generation: int) -> StopReceipt:
        with self._exclusive_stop():
            with self._lock:
                row = self._get(operation_id)
                if row is None or row[0] != generation or row[2] != session_id:
                    raise RuntimeError("STOP target changed; shutdown not confirmed")
                if row[1] == "stopped":
                    verified = self._db.execute(
                        "SELECT stop_verified FROM media_operations WHERE id=?",
                        (operation_id,),
                    ).fetchone()[0]
                    if verified == 1:
                        # Durable STOP is already recorded. A prior process
                        # may have committed just before in-memory cleanup.
                        acknowledge = getattr(
                            self._backend, "acknowledge_durable_stop", None
                        )
                        if callable(acknowledge):
                            acknowledge(operation_id, session_id, generation)
                        return StopReceipt(operation_id, session_id, generation, True, True)
                    # A pre-migration stopped row is not a STOP receipt.
                    self._write(operation_id, generation, "uncertain", session_id)
                    row = self._get(operation_id)
                if row[1] not in ("active", "stopping", "uncertain"):
                    raise RuntimeError("STOP target unresolved; shutdown not confirmed")
                self._write(operation_id, generation, "stopping", session_id)
                preassigned, actual_confirmed = self._db.execute(
                    """SELECT preassigned_session_id,actual_stop_confirmed
                       FROM media_operations WHERE id=? AND generation=?""",
                    (operation_id, generation),
                ).fetchone()
                mismatch = preassigned is not None and preassigned != session_id
            try:
                if actual_confirmed:
                    if not mismatch:
                        raise RuntimeError("invalid durable STOP confirmation state")
                    # Reconstruct ONLY the persisted, verified actual sender
                    # receipt. Never claim the promised sender stopped.
                    receipt = StopReceipt(operation_id, session_id, generation, True, True)
                else:
                    receipt = self._backend.stop(operation_id, session_id, generation)
                    if not self._validate(receipt, operation_id, session_id, generation):
                        raise RuntimeError("remote audio/video STOP not confirmed")
                    if mismatch:
                        # Persist the actual sender's receipt before attempting
                        # the promised ID. A restart must retain partial progress.
                        with self._lock:
                            self._db.execute("BEGIN IMMEDIATE")
                            try:
                                if self._get(operation_id) != (generation, "stopping", session_id):
                                    raise RuntimeError("STOP target changed during compensation")
                                self._db.execute(
                                    """UPDATE media_operations SET actual_stop_confirmed=1
                                       WHERE id=? AND generation=?""",
                                    (operation_id, generation),
                                )
                                self._db.execute("COMMIT")
                            except BaseException:
                                if self._db.in_transaction:
                                    self._db.execute("ROLLBACK")
                                raise
                if mismatch:
                    promised_receipt = self._backend.stop(
                        operation_id, preassigned, generation
                    )
                    if not self._validate(
                        promised_receipt, operation_id, preassigned, generation
                    ):
                        raise RuntimeError("preassigned remote audio/video STOP not confirmed")
            except Exception:
                with self._lock:
                    self._db.execute("BEGIN IMMEDIATE")
                    try:
                        if self._get(operation_id) == (generation, "stopping", session_id):
                            self._write(operation_id, generation, "uncertain", session_id)
                        self._db.execute("COMMIT")
                    except BaseException:
                        if self._db.in_transaction:
                            self._db.execute("ROLLBACK")
                        raise
                raise
            with self._lock:
                self._db.execute("BEGIN IMMEDIATE")
                try:
                    if self._get(operation_id) != (generation, "stopping", session_id):
                        raise RuntimeError("STOP target changed before confirmation")
                    self._write(operation_id, generation, "stopped", session_id)
                    # Commit the verified BOTH-track receipt atomically with
                    # stopped; a crash cannot expose an unverified success.
                    self._db.execute(
                        "UPDATE media_operations SET stop_verified=1 WHERE id=? AND generation=?",
                        (operation_id, generation),
                    )
                    self._db.execute("COMMIT")
                except BaseException:
                    if self._db.in_transaction:
                        self._db.execute("ROLLBACK")
                    raise
            # Only now can the sender evict its in-process idempotent
            # receipt(s): the canonical SQLite ledger is durably authoritative.
            # Keep both targets distinct for mismatched preassigned CREATE.
            acknowledge = getattr(self._backend, "acknowledge_durable_stop", None)
            if callable(acknowledge):
                acknowledge(operation_id, session_id, generation)
                if mismatch:
                    acknowledge(operation_id, preassigned, generation)
            return receipt

    def request_shutdown(self) -> None:
        """Immediately fence new CREATE and signal every live provider transport."""
        self._closing.set()
        with self._create_cancellations_lock:
            for cancellation in self._create_cancellations.values():
                cancellation.set()

    def begin_shutdown(self, deadline: float | None = None) -> bool:
        """Fence CREATE immediately, then durably quarantine through one deadline.

        This uses the same SQLite ledger and RLock as normal STOP; a competing
        writer or in-process lock cannot consume the shutdown grace period.
        An unsuccessful write never implies that remote senders stopped.
        """
        self.request_shutdown()
        if deadline is None:
            self.quarantine_after_restart()
            return True
        left = deadline - time.monotonic()
        if left <= 0 or not self._lock.acquire(timeout=left):
            return False
        try:
            previous = self._db.execute("PRAGMA busy_timeout").fetchone()[0]
            left = deadline - time.monotonic()
            if left <= 0:
                return False
            # Restrict SQLite's normal 5-second busy waiter to the remaining
            # global shutdown budget; restore the original setting afterward.
            self._db.execute(f"PRAGMA busy_timeout={max(0, int(left * 1000))}")
            try:
                self.quarantine_after_restart()
            except sqlite3.OperationalError as error:
                if error.sqlite_errorcode in (sqlite3.SQLITE_BUSY, sqlite3.SQLITE_LOCKED):
                    return False
                raise
            finally:
                self._db.execute(f"PRAGMA busy_timeout={previous}")
            return True
        finally:
            self._lock.release()

    def revoke_all_bounded(self, timeout: float) -> bool:
        """Revoke ALL operations and await verified STOP within one deadline."""
        if not 0 < timeout <= 60:
            raise ValueError("invalid shutdown deadline")
        deadline = time.monotonic() + timeout
        if not self.begin_shutdown(deadline):
            return False
        with self._lock:
            operations = self._db.execute(
                """SELECT id,generation FROM media_operations
                   WHERE phase != 'stopped'
                      OR (session_id IS NOT NULL AND stop_verified=0)"""
            ).fetchall()
        for operation_id, generation in operations:
            left = deadline - time.monotonic()
            if left <= 0:
                break
            try:
                self.cancel_bounded(operation_id, generation, min(left, 1.0))
            except Exception:
                # Keep compensating other operations; failed STOP stays
                # quarantined and retryable in the one durable ledger.
                pass
        # STOP can finish *after* the first 1-second HTTP timeout. The
        # remaining grace budget belongs to late verified receipts.
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                return False
            if not self._lock.acquire(timeout=left):
                return False
            try:
                unresolved = self._db.execute(
                    """SELECT 1 FROM media_operations
                       WHERE phase != 'stopped'
                          OR (session_id IS NOT NULL AND stop_verified=0)
                       LIMIT 1"""
                ).fetchone()
            finally:
                self._lock.release()
            if unresolved is None:
                return True
            time.sleep(min(0.025, max(0, deadline - time.monotonic())))

    def quarantine_after_restart(self):
        """Fence every sender identity inherited without a live backend handle.

        A fresh process cannot assume that an old aiortc peer is gone. Keep
        known session IDs for authenticated STOP; never repeat CREATE and
        never return an old 'active' session as an idempotent new admission.
        """
        with self._lock:
            self._db.execute("BEGIN IMMEDIATE")
            try:
                self._db.execute(
                    """UPDATE media_operations SET phase='uncertain',
                       session_id=COALESCE(session_id, preassigned_session_id)
                       WHERE phase != 'stopped'
                          OR (phase='stopped' AND session_id IS NOT NULL
                              AND stop_verified=0)"""
                )
                self._db.execute("COMMIT")
            except BaseException:
                if self._db.in_transaction:
                    self._db.execute("ROLLBACK")
                raise

    def reconcile(self):
        """Retry each known STOP independently; never replay CREATE.

        A permanently unreachable sender must not starve later eligible STOPs.
        Each failed sender keeps its durable uncertain row and the whole pass
        remains unsuccessful so the caller can report/retry reconciliation.

        When a failed CREATE acknowledgement could not persist CANCEL because
        SQLite was locked, request_shutdown() immediately denies new CREATEs.
        The SAME canonical reconciler must later quarantine any still-active
        sender before STOP. Merely selecting uncertain/stopping rows would
        leave that sender alive indefinitely after SQLite becomes writable.
        """
        if self._closing.is_set():
            self.quarantine_after_restart()
        first_failure = None
        # GPU inference may fail AFTER the worker has acknowledged PCM.
        # Discover such live senders through the existing provider, then
        # persist their CANCEL in this same SQLite owner before physical STOP.
        # No second session registry or remote STOP authority is created.
        observe_failed = getattr(self._backend, "failed_renderer_sessions", None)
        if callable(observe_failed) and not self._closing.is_set():
            try:
                failed_sessions = observe_failed()
            except Exception as error:
                failed_sessions = ()
                first_failure = error
            for operation_id, generation in failed_sessions:
                try:
                    self.cancel(operation_id, generation, wait_for_stop=False)
                except Exception as error:
                    if first_failure is None:
                        first_failure = error
        with self._lock:
            pending = self._db.execute(
                "SELECT id,generation,session_id FROM media_operations WHERE phase IN ('stopping','uncertain')"
            ).fetchall()
        for operation_id, generation, session_id in pending:
            if not session_id:
                continue
            try:
                self._finish_stop(operation_id, session_id, generation)
            except Exception as error:
                if first_failure is None:
                    first_failure = error
        if first_failure is not None:
            # Preserve the established provider error contract for callers
            # and recovery tests, after attempting every known STOP target.
            raise first_failure

    def unresolved_operations(self) -> tuple[tuple[str, int, str], ...]:
        """Return operation identities that still require verified remote STOP.

        This reads the existing durable ledger. It is not a second registry and
        deliberately omits provider session tokens or sender credentials.
        """
        with self._lock:
            rows = self._db.execute(
                """SELECT id, generation, phase FROM media_operations
                   WHERE phase != 'stopped'
                      OR (session_id IS NOT NULL AND stop_verified=0)
                   ORDER BY id"""
            ).fetchall()
        return tuple(
            (operation_id, generation, phase)
            for operation_id, generation, phase in rows
        )

    def status(self, operation_id: str):
        with self._lock:
            return self._get(operation_id)

    def status_with_stop_confirmation(self, operation_id: str):
        """Read status and durable dual-track STOP evidence atomically.

        A legacy or interrupted row may say 'stopped' with an allocated sender
        but lack stop_verified=1. Never report that row as remotely stopped.
        The single SQLite SELECT keeps phase and evidence in one snapshot.
        """
        with self._lock:
            row = self._db.execute(
                """SELECT generation,phase,session_id,stop_verified
                   FROM media_operations WHERE id=?""",
                (operation_id,),
            ).fetchone()
        if row is None:
            return None
        generation, phase, session_id, verified = row
        return (
            generation, phase, session_id,
            phase == "stopped" and session_id is not None and verified == 1,
        )
