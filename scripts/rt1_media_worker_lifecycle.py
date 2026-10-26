"""RT1 worker-side durable allocation and sender shutdown coordinator.

Not a WebRTC implementation. The caller must supply an actual SenderBackend
that does not acknowledge stop until RTP audio and video senders are stopped.
"""
from __future__ import annotations

import sqlite3
import threading
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
            previous = self._get(operation_id)
            if previous is not None:
                if previous[0] == generation and previous[1] == "active":
                    return previous[2]
                raise RuntimeError("operation already exists or has been cancelled")
            # This core deliberately admits at most ONE remote publisher at once.
            # Reconcile unknown outcomes before permitting a different operation.
            if self._db.execute(
                "SELECT 1 FROM media_operations WHERE phase != 'stopped' LIMIT 1"
            ).fetchone():
                raise RuntimeError("worker has unresolved media allocation")
            self._write(operation_id, generation, "creating", None)
        try:
            session_id = self._backend.allocate(operation_id, generation)
            if not session_id:
                raise RuntimeError("backend supplied an empty session identifier")
        except BaseException:
            with self._lock:
                row = self._get(operation_id)
                # Failed creation may have allocated remotely. Never retry CREATE.
                if row and row[1] == "creating":
                    self._write(operation_id, generation, "uncertain", None)
            raise
        with self._lock:
            row = self._get(operation_id)
            if row and row[1] == "creating":
                self._write(operation_id, generation, "active", session_id)
                return session_id
            # Cancellation won while backend was starting; preserve the session
            # address for compensating STOP, never admit it as active.
            self._write(operation_id, generation, "stopping", session_id)
        self._finish_stop(operation_id, session_id, generation)
        raise RuntimeError("operation revoked during allocation")

    def cancel(self, operation_id: str, generation: int) -> StopReceipt | None:
        with self._lock:
            row = self._get(operation_id)
            if row is None or row[0] != generation:
                raise KeyError("unknown or stale operation")
            _, phase, session_id = row
            if phase == "stopped":
                return StopReceipt(operation_id, session_id or "", generation, True, True)
            if phase == "creating":
                self._write(operation_id, generation, "stopping", None)
                # CREATE completion owns the eventual compensating STOP. When
                # CREATE never completes, caller must keep operation unresolved.
                return None
            if not session_id:
                self._write(operation_id, generation, "uncertain", None)
                return None
            self._write(operation_id, generation, "stopping", session_id)
        return self._finish_stop(operation_id, session_id, generation)

    def _finish_stop(self, operation_id: str, session_id: str, generation: int) -> StopReceipt:
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
