"""Real aiortc media sender backend for the canonical durable MediaLifecycle.

This backend owns *only* live sender objects. All CREATE/CANCEL/STOP decisions
belong to rt1_media_worker_lifecycle.MediaLifecycle's SQLite operation ledger.
It supplies generated audio/video tracks as a transport diagnostic, not a GPU
Persona renderer or authorization to expose visitors.
"""
from __future__ import annotations

import asyncio
import concurrent.futures
import threading
from dataclasses import dataclass

from rt1_media_worker_lifecycle import StopReceipt


@dataclass
class _Session:
    peer: object
    audio: object
    video: object
    operation_id: str
    generation: int


class AiortcSenderBackend:
    """Dedicated event loop for the synchronous canonical worker coordinator."""

    def __init__(self, timeout: float = 8.0):
        if timeout <= 0:
            raise ValueError("timeout must be positive")
        self._timeout = timeout
        self._loop = asyncio.new_event_loop()
        self._thread = threading.Thread(target=self._run_loop, name="rt1-aiortc", daemon=True)
        self._lock = threading.RLock()
        self._sessions: dict[str, _Session] = {}
        self._thread.start()

    def _run_loop(self):
        asyncio.set_event_loop(self._loop)
        self._loop.run_forever()

    def _call(self, coroutine):
        future = asyncio.run_coroutine_threadsafe(coroutine, self._loop)
        try:
            return future.result(self._timeout)
        except concurrent.futures.TimeoutError as exc:
            # Do not let a timed-out CREATE/ANSWER/STOP continue unbounded in
            # the background. Cancellation is best-effort: even after cancel,
            # remote outcome remains UNCERTAIN until a real STOP receipt.
            future.cancel()
            raise RuntimeError("WebRTC operation timed out; STOP unconfirmed") from exc

    def allocate(self, operation_id: str, generation: int) -> str:
        return self._call(self._allocate(operation_id, generation))

    async def _allocate(self, operation_id: str, generation: int) -> str:
        from aiortc import AudioStreamTrack, RTCPeerConnection, VideoStreamTrack
        session_id = f"rt1-{operation_id}-{generation}"
        peer = RTCPeerConnection()
        audio = AudioStreamTrack()
        video = VideoStreamTrack()
        try:
            peer.addTrack(audio)
            peer.addTrack(video)
            await peer.setLocalDescription(await peer.createOffer())
            with self._lock:
                if session_id in self._sessions:
                    raise RuntimeError("WebRTC session already allocated")
                self._sessions[session_id] = _Session(
                    peer, audio, video, operation_id, generation
                )
            return session_id
        except BaseException:
            audio.stop()
            video.stop()
            await peer.close()
            raise

    def offer(self, operation_id: str, session_id: str, generation: int) -> dict:
        return self._call(self._offer(operation_id, session_id, generation))

    async def _offer(self, operation_id: str, session_id: str, generation: int) -> dict:
        with self._lock:
            entry = self._sessions.get(session_id)
        if not entry or (entry.operation_id, entry.generation) != (operation_id, generation):
            raise RuntimeError("unknown or revoked WebRTC session")
        description = entry.peer.localDescription
        if description is None or entry.peer.connectionState == "closed":
            raise RuntimeError("WebRTC offer not available")
        return {"type": description.type, "sdp": description.sdp}

    def answer(self, operation_id: str, session_id: str, generation: int, sdp: str):
        return self._call(self._answer(operation_id, session_id, generation, sdp))

    async def _answer(self, operation_id: str, session_id: str, generation: int, sdp: str):
        from aiortc import RTCSessionDescription
        with self._lock:
            entry = self._sessions.get(session_id)
        if not entry or (entry.operation_id, entry.generation) != (operation_id, generation):
            raise RuntimeError("unknown or revoked WebRTC session")
        if entry.peer.remoteDescription is not None:
            raise RuntimeError("WebRTC answer already accepted")
        await entry.peer.setRemoteDescription(RTCSessionDescription(sdp=sdp, type="answer"))

    def stop(self, operation_id: str, session_id: str, generation: int) -> StopReceipt:
        return self._call(self._stop(operation_id, session_id, generation))

    async def _stop(self, operation_id: str, session_id: str, generation: int) -> StopReceipt:
        with self._lock:
            entry = self._sessions.get(session_id)
        if not entry or (entry.operation_id, entry.generation) != (operation_id, generation):
            # Never fabricate STOP for a session that may live on another worker.
            raise RuntimeError("remote WebRTC sender missing; STOP unconfirmed")
        entry.audio.stop()
        entry.video.stop()
        await entry.peer.close()
        if (entry.audio.readyState != "ended" or entry.video.readyState != "ended"
                or entry.peer.connectionState != "closed"):
            raise RuntimeError("WebRTC sender shutdown not independently confirmed")
        with self._lock:
            self._sessions.pop(session_id, None)
        return StopReceipt(operation_id, session_id, generation, True, True)

    def shutdown(self):
        with self._lock:
            pending = list(self._sessions.values())
        for entry in pending:
            try:
                self.stop(entry.operation_id, f"rt1-{entry.operation_id}-{entry.generation}", entry.generation)
            except Exception:
                # Caller must inspect its durable ledger and retry reconciliation.
                pass
        self._loop.call_soon_threadsafe(self._loop.stop)
        self._thread.join(timeout=self._timeout)
