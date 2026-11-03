"""RT1 worker-side durable allocation and sender shutdown coordinator.

Not a WebRTC implementation. The caller must supply an actual SenderBackend
that does not acknowledge stop until RTP audio and video senders are stopped.
"""
from __future__ import annotations

import os
import sqlite3
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
    def allocate(self, operation_id: str, generation: int) -> str:
        """Create one session; return its opaque id. Can block."""

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
            self._db.execute("COMMIT")
        except BaseException:
            if self._db.in_transaction:
                self._db.execute("ROLLBACK")
            raise
        self._lock = threading.RLock()
        # A timed-out CREATE still owns its provider thread until it exits.
        # Bound outstanding allocations instead of spawning one per HTTP PUT.
        self._create_gate = threading.BoundedSemaphore(1)
        # A hung STOP owns one worker thread until the backend returns.
        # Repeated HTTP DELETEs must not accumulate unbounded STOP threads.
        self._stop_gate = threading.BoundedSemaphore(1)
        self._stop_lock_path = str(path) + ".stop.lock"
        self._backend = backend

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
        """Bound the HTTP CREATE wait without abandoning a late provider result.

        The *same* durable create() continues in one daemon thread. On timeout
        CANCEL is committed before returning, without waiting for a hung
        provider STOP. A late allocated ID is compensated by create(); if the
        provider never returns, the durable unknown-ID quarantine stays closed.
        The periodic canonical reconciler owns STOP if CREATE already finished.
        """
        if not 0 < timeout <= 60:
            raise ValueError("invalid CREATE timeout")
        if not self._create_gate.acquire(blocking=False):
            raise RuntimeError("another provider CREATE remains in flight")
        outcome = Queue(maxsize=1)

        def allocate():
            try:
                outcome.put((True, self.create(operation_id, generation)))
            except Exception as error:
                outcome.put((False, error))
            finally:
                self._create_gate.release()

        try:
            threading.Thread(
                target=allocate, name="rt1-bounded-create", daemon=True
            ).start()
        except Exception:
            self._create_gate.release()
            raise
        try:
            succeeded, result = outcome.get(timeout=timeout)
        except Empty:
            # CREATE may have completed and committed 'active' immediately
            # before the response deadline. A tombstone alone would leave
            # that known remote sender running until the next reconciliation.
            # Persist cancellation first, then initiate a separately bounded
            # compensating STOP when its identity is already known. If CREATE
            # is still in flight, create() owns the late-ID compensation.
            # Neither outcome turns a timed-out CREATE into admission.
            try:
                self.cancel_bounded(operation_id, generation, min(timeout, 0.05))
            except Exception:
                # STOP failure remains quarantined in the durable ledger;
                # preserve the original CREATE deadline error to the caller.
                pass
            raise RuntimeError("CREATE deadline exceeded; remote STOP unconfirmed") from None
        if succeeded:
            return result
        raise result

    def create(self, operation_id: str, generation: int) -> str:
        if not operation_id or generation < 1:
            raise ValueError("invalid operation identity")
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
                    "SELECT 1 FROM media_operations WHERE phase != 'stopped' LIMIT 1"
                ).fetchone():
                    raise RuntimeError("worker has unresolved media allocation")
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
            session_id = self._backend.allocate(operation_id, generation)
            if not session_id:
                raise RuntimeError("backend supplied an empty session identifier")
        except BaseException as create_error:
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
                    # A failed CREATE may still have allocated remotely. Do not
                    # retry it or discard an already committed cancellation.
                    if row and row[0] == generation and row[1] in ("creating", "stopping", "uncertain") and row[2] is None:
                        # CANCEL may have committed while CREATE was in flight.
                        # A lost CREATE reply may still have a known, stable
                        # sender ID; otherwise retain an unknown-id quarantine.
                        # Neither outcome is a verified STOP.
                        self._write(operation_id, generation, "uncertain", cleanup_id)
                    self._db.execute("COMMIT")
                except BaseException:
                    if self._db.in_transaction:
                        self._db.execute("ROLLBACK")
                    raise
            # A provider can allocate successfully and lose its CREATE reply.
            # When the backend supplies an authoritative preassigned identity
            # OR a verified late allocation ID, compensate NOW instead of
            # waiting for a worker restart or the
            # periodic reconciler. Keep the original CREATE error: even a
            # verified STOP must never turn a failed allocation into admission.
            # An unsuccessful STOP remains durable and retryable as 'uncertain'.
            if cleanup_id is not None:
                try:
                    self._finish_stop(operation_id, cleanup_id, generation)
                except Exception:
                    # The durable uncertain row remains the retry handle.
                    pass
            raise
        if recovery_id is not None and session_id != recovery_id:
            # The backend violated its authoritative identity contract. The
            # returned sender may already be transmitting: persist its ACTUAL
            # ID before attempting compensation, including across restart.
            # Retain the original preassigned ID as a second possible target.
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
            # Best-effort immediate STOP for the newly discovered sender.
            # Even a valid receipt cannot prove the preassigned ID was inert;
            # _finish_stop deliberately retains the identity quarantine.
            try:
                self._finish_stop(operation_id, session_id, generation)
            except Exception as error:
                raise RuntimeError(
                    "backend CREATE identity mismatch; compensating STOP unconfirmed"
                ) from error
            raise RuntimeError("backend CREATE identity mismatch; both possible sender STOPs confirmed")
        with self._lock:
            # CREATE completion and CANCEL must have one SQLite linearization
            # point across *processes*. The Python RLock alone is insufficient:
            # without this transaction, a stale 'creating' read can overwrite
            # a committed CANCEL with 'active' and expose revoked media.
            self._db.execute("BEGIN IMMEDIATE")
            try:
                row = self._get(operation_id)
                if row and row[0] == generation and row[1] == "creating":
                    self._write(operation_id, generation, "active", session_id)
                    self._db.execute("COMMIT")
                    return session_id
                # Cancellation won while the backend was starting. Persist the
                # returned session address for compensating STOP, never admit it.
                self._write(operation_id, generation, "stopping", session_id)
                self._db.execute("COMMIT")
            except BaseException:
                if self._db.in_transaction:
                    self._db.execute("ROLLBACK")
                raise
        self._finish_stop(operation_id, session_id, generation)
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
                    self._db.execute("COMMIT")
                    if session_id is None:
                        return None
                    return StopReceipt(operation_id, session_id, generation, True, True)
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
                    # Only prior verified STOPs can persist a session id here.
                    return StopReceipt(operation_id, session_id, generation, True, True)
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
                    self._db.execute("COMMIT")
                except BaseException:
                    if self._db.in_transaction:
                        self._db.execute("ROLLBACK")
                    raise
            return receipt

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
                       WHERE phase != 'stopped'"""
                )
                self._db.execute("COMMIT")
            except BaseException:
                if self._db.in_transaction:
                    self._db.execute("ROLLBACK")
                raise

    def reconcile(self):
        """Replay STOP only for known session IDs; NEVER replay CREATE."""
        with self._lock:
            pending = self._db.execute(
                "SELECT id,generation,session_id FROM media_operations WHERE phase IN ('stopping','uncertain')"
            ).fetchall()
        for operation_id, generation, session_id in pending:
            if session_id:
                self._finish_stop(operation_id, session_id, generation)

    def status(self, operation_id: str):
        with self._lock:
            return self._get(operation_id)
