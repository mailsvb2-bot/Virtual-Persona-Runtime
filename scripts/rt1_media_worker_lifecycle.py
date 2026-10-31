"""RT1 worker-side durable allocation and sender shutdown coordinator.

Not a WebRTC implementation. The caller must supply an actual SenderBackend
that does not acknowledge stop until RTP audio and video senders are stopped.
"""
from __future__ import annotations

import os
import sqlite3
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
            CHECK(phase IN ('creating', 'active', 'stopping', 'stopped', 'uncertain'))
        )""")
        self._lock = threading.RLock()
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

    def create(self, operation_id: str, generation: int) -> str:
        if not operation_id or generation < 1:
            raise ValueError("invalid operation identity")
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
                self._db.execute("COMMIT")
            except BaseException:
                if self._db.in_transaction:
                    self._db.execute("ROLLBACK")
                raise
        try:
            session_id = self._backend.allocate(operation_id, generation)
            if not session_id:
                raise RuntimeError("backend supplied an empty session identifier")
        except BaseException:
            with self._lock:
                # A different worker may CANCEL between a read and a write.
                # Never overwrite its durable tombstone with an old CREATE result.
                self._db.execute("BEGIN IMMEDIATE")
                try:
                    row = self._get(operation_id)
                    # A failed CREATE may still have allocated remotely. Do not
                    # retry it or discard an already committed cancellation.
                    if row and row[0] == generation and row[1] in ("creating", "stopping") and row[2] is None:
                        # CANCEL may have committed while CREATE was in flight.
                        # A failed/timeout CREATE has no verifiable sender ID;
                        # keep the cancellation fenced as UNCERTAIN, never STOPPED.
                        self._write(operation_id, generation, "uncertain", None)
                    self._db.execute("COMMIT")
                except BaseException:
                    if self._db.in_transaction:
                        self._db.execute("ROLLBACK")
                    raise
            raise
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

    def cancel(self, operation_id: str, generation: int) -> StopReceipt | None:
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
                    # Only a prior verified STOP can persist a session id here.
                    return StopReceipt(operation_id, session_id, generation, True, True)
                if row[1] not in ("active", "stopping", "uncertain"):
                    raise RuntimeError("STOP target unresolved; shutdown not confirmed")
                self._write(operation_id, generation, "stopping", session_id)
            try:
                receipt = self._backend.stop(operation_id, session_id, generation)
            except Exception:
                with self._lock:
                    self._write(operation_id, generation, "uncertain", session_id)
                raise
            with self._lock:
                if not self._validate(receipt, operation_id, session_id, generation):
                    self._write(operation_id, generation, "uncertain", session_id)
                    raise RuntimeError("remote audio/video STOP not confirmed")
                self._write(operation_id, generation, "stopped", session_id)
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
                    "UPDATE media_operations SET phase='uncertain' WHERE phase != 'stopped'"
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
