"""Fail-closed lifecycle owner for a self-hosted avatar media worker.

Wire the real GPU/WebRTC implementation through MediaAllocation. This module
does not fabricate audio/video STOP receipts: each sender must acknowledge stop.
Use a durable operation ledger in production: in-memory ownership alone cannot
recover outstanding CREATE operations after a worker restart.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from threading import Event, Lock
from time import monotonic
from typing import Callable, Protocol


class Sender(Protocol):
    def stop(self) -> None: ...
    def wait_stopped(self, timeout: float) -> bool: ...


@dataclass
class MediaAllocation:
    session_id: str
    audio: Sender
    video: Sender


@dataclass
class Operation:
    cancelled: Event = field(default_factory=Event)
    complete: Event = field(default_factory=Event)
    allocation: MediaAllocation | None = None
    failure: BaseException | None = None
    stopping: bool = False
    stopped: bool = False


class LifecycleError(RuntimeError):
    pass


class MediaLifecycle:
    """A single owner for CREATE, CANCEL and STOP, including startup races."""

    def __init__(self) -> None:
        self._guard = Lock()
        self._operations: dict[str, Operation] = {}
        self._sessions: dict[str, str] = {}

    def create(self, operation_id: str, allocate: Callable[[Event], MediaAllocation]) -> MediaAllocation:
        if not operation_id or len(operation_id) > 128:
            raise LifecycleError("invalid operation id")
        with self._guard:
            if operation_id in self._operations:
                raise LifecycleError("duplicate operation id; never allocate twice")
            operation = Operation()
            self._operations[operation_id] = operation
        try:
            allocation = allocate(operation.cancelled)
            if not allocation.session_id:
                raise LifecycleError("missing session id")
            with self._guard:
                if allocation.session_id in self._sessions:
                    raise LifecycleError("duplicate session id")
                operation.allocation = allocation
                self._sessions[allocation.session_id] = operation_id
            if operation.cancelled.is_set():
                self._stop_operation(operation, timeout=2.0)
                raise LifecycleError("CREATE cancelled")
            return allocation
        except BaseException as exc:
            operation.failure = exc
            # If allocate did not return a handle, it MUST itself guarantee that
            # no sender remains alive. No generic cleanup can find that sender.
            raise
        finally:
            operation.complete.set()

    def cancel(self, operation_id: str, timeout: float) -> dict[str, object]:
        with self._guard:
            operation = self._operations.get(operation_id)
            if operation is None:
                # Tombstone: a late CREATE with this id cannot allocate.
                operation = Operation()
                operation.cancelled.set()
                operation.complete.set()
                operation.stopped = True
                self._operations[operation_id] = operation
                return {"operation_id": operation_id, "audio_stopped": True, "video_stopped": True}
            operation.cancelled.set()
        if not operation.complete.wait(timeout):
            raise LifecycleError("CREATE still running: remote STOP is unconfirmed")
        return self._stop_operation(operation, timeout)

    def stop(self, session_id: str, timeout: float) -> dict[str, object]:
        with self._guard:
            operation_id = self._sessions.get(session_id)
            if operation_id is None:
                raise LifecycleError("unknown session: STOP unconfirmed")
            operation = self._operations[operation_id]
            operation.cancelled.set()
        return self._stop_operation(operation, timeout)

    def _stop_operation(self, operation: Operation, timeout: float) -> dict[str, object]:
        with self._guard:
            allocation = operation.allocation
            if allocation is None:
                if operation.stopped:
                    return {"audio_stopped": True, "video_stopped": True}
                raise LifecycleError("allocation not resolved: STOP unconfirmed")
            if operation.stopping and not operation.stopped:
                raise LifecycleError("concurrent STOP still unconfirmed")
            if operation.stopped:
                return {"session_id": allocation.session_id, "audio_stopped": True, "video_stopped": True}
            operation.stopping = True
        try:
            deadline = monotonic() + max(0.0, timeout)
            # Attempt both STOP operations even when the first sender fails.
            # Neither sender may remain running merely because its peer failed.
            stop_failed = False
            for sender in (allocation.audio, allocation.video):
                try:
                    sender.stop()
                except Exception:
                    stop_failed = True
            # One shared deadline, not a fresh timeout for each sender.
            audio_stopped = allocation.audio.wait_stopped(max(0.0, deadline - monotonic()))
            video_stopped = allocation.video.wait_stopped(max(0.0, deadline - monotonic()))
            if stop_failed or not (audio_stopped and video_stopped):
                raise LifecycleError("audio/video STOP unconfirmed: sender shutdown not independently verified")
            with self._guard:
                operation.stopped = True
            return {"session_id": allocation.session_id, "audio_stopped": True, "video_stopped": True}
        finally:
            with self._guard:
                operation.stopping = False
