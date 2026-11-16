"""Real aiortc media sender backend for the canonical durable MediaLifecycle.

This backend owns *only* live sender objects. All CREATE/CANCEL/STOP decisions
belong to rt1_media_worker_lifecycle.MediaLifecycle's SQLite operation ledger.
It supplies generated audio/video tracks as a transport diagnostic, not a GPU
Persona renderer or authorization to expose visitors.
"""
from __future__ import annotations

import asyncio
import concurrent.futures
import json
import logging
import os
import threading
import time
from collections import deque
from fractions import Fraction
from dataclasses import dataclass, field

from rt1_media_worker_lifecycle import StopReceipt


LOG = logging.getLogger(__name__)


def parse_ice_servers(raw: str | None) -> list[dict]:
    """Single canonical ICE configuration for worker and visitor.

    TURN credentials are short-lived access credentials, not long-term worker
    API secrets. Malformed or insecure ICE settings fail startup closed.
    """
    if raw is None or not raw.strip():
        return []
    servers = json.loads(raw)
    if not isinstance(servers, list) or len(servers) > 8:
        raise ValueError("RT1 ICE must be an array of at most 8 servers")
    result = []
    for item in servers:
        if not isinstance(item, dict) or set(item) - {"urls", "username", "credential"}:
            raise ValueError("invalid ICE server object")
        urls = item.get("urls")
        if not isinstance(urls, list) or not 1 <= len(urls) <= 8:
            raise ValueError("invalid ICE server URL list")
        if any(not isinstance(url, str) or not url.startswith(
            ("stun:", "stuns:", "turn:", "turns:")
        ) or len(url) > 1024 for url in urls):
            raise ValueError("invalid ICE URL")
        username = item.get("username")
        credential = item.get("credential")
        if (username is None) != (credential is None):
            raise ValueError("incomplete ICE credentials")
        if any(value is not None and (
            not isinstance(value, str) or not value or len(value) > 512
        ) for value in (username, credential)):
            raise ValueError("invalid ICE credentials")
        if any(url.startswith(("turn:", "turns:")) for url in urls) and not username:
            raise ValueError("TURN requires ephemeral credentials")
        result.append({"urls": urls, "username": username, "credential": credential})
    return result


def _make_pcm_audio_track():
    """One bounded PCM16 mono 16 kHz source for the existing aiortc RTP sender.

    Browser delivery consumes the same track from CREATE until verified STOP.
    No second WebRTC peer or voice executor is created.
    """
    from aiortc import AudioStreamTrack
    from aiortc.mediastreams import MediaStreamError
    from av import AudioFrame

    class PcmAudioTrack(AudioStreamTrack):
        SAMPLE_RATE = 16_000
        FRAME_SAMPLES = 320  # 20 ms, 640 bytes
        MAX_PENDING_FRAMES = 100  # 2 seconds, bounded producer
        FRAME_BYTES = FRAME_SAMPLES * 2

        def __init__(self):
            super().__init__()
            self._frames = deque()
            self._guard = threading.Lock()
            self._pts = 0
            self._started = None

        def push_pcm(self, pcm: bytes):
            if not pcm or len(pcm) % self.FRAME_BYTES:
                raise ValueError("PCM must contain complete 20ms mono 16k frames")
            count = len(pcm) // self.FRAME_BYTES
            with self._guard:
                if self.readyState != "live":
                    raise RuntimeError("remote audio sender stopped")
                if len(self._frames) + count > self.MAX_PENDING_FRAMES:
                    raise RuntimeError("remote audio sender backpressure")
                self._frames.extend(
                    pcm[offset:offset + self.FRAME_BYTES]
                    for offset in range(0, len(pcm), self.FRAME_BYTES)
                )

        async def recv(self):
            if self.readyState != "live":
                raise MediaStreamError
            if self._started is None:
                self._started = time.monotonic()
            await asyncio.sleep(max(
                0.0, self._started + self._pts / self.SAMPLE_RATE - time.monotonic()
            ))
            if self.readyState != "live":
                raise MediaStreamError
            with self._guard:
                payload = self._frames.popleft() if self._frames else bytes(self.FRAME_BYTES)
            frame = AudioFrame(format="s16", layout="mono", samples=self.FRAME_SAMPLES)
            frame.planes[0].update(payload)
            frame.sample_rate = self.SAMPLE_RATE
            frame.pts = self._pts
            frame.time_base = Fraction(1, self.SAMPLE_RATE)
            self._pts += self.FRAME_SAMPLES
            return frame

        def stop(self):
            with self._guard:
                self._frames.clear()
            super().stop()

    return PcmAudioTrack()


@dataclass
class _Session:
    peer: object
    audio: object
    video: object
    operation_id: str
    generation: int
    gate: asyncio.Lock = field(default_factory=asyncio.Lock)
    closed: bool = False


class AiortcSenderBackend:
    """Dedicated event loop for the synchronous canonical worker coordinator."""

    def __init__(self, timeout: float = 8.0, *, video_renderer=None):
        """Optional trusted in-process video source; never a second session owner.

        The default diagnostic track cannot authorize public Persona links.
        A production GPU renderer must be passed explicitly by the worker
        composition root and must independently prove model readiness.
        """
        if timeout <= 0:
            raise ValueError("timeout must be positive")
        self._video_renderer = video_renderer
        self._timeout = timeout
        self._ice_servers = parse_ice_servers(os.environ.get('RT1_ICE_SERVERS_JSON'))
        self._loop = asyncio.new_event_loop()
        self._thread = threading.Thread(target=self._run_loop, name="rt1-aiortc", daemon=True)
        self._lock = threading.RLock()
        self._sessions: dict[str, _Session] = {}
        # A STOP can finish after its synchronous caller times out. Retain
        # ONLY verified in-process receipts so durable reconciliation can
        # retry without mistaking an already-closed sender for an unknown one.
        # A process restart has no such evidence and must remain fail-closed.
        self._completed_stops: dict[str, StopReceipt] = {}
        self._thread.start()

    def persona_video_ready(self) -> bool:
        """A diagnostic VideoStreamTrack can never certify Persona readiness.

        Only an explicitly injected renderer with a successful real-model
        readiness probe can authorize publication. Probe failures deny access;
        the durable lifecycle checks readiness again before CREATE.
        """
        if self._video_renderer is None:
            return False
        try:
            return self._video_renderer.ready() is True
        except Exception:
            return False

    def _run_loop(self):
        asyncio.set_event_loop(self._loop)
        self._loop.run_forever()

    def _fence_cancelled_create(
        self,
        cancel_requested: threading.Event | None,
        cancel_identity: tuple[str, int] | None,
    ):
        """Fence a CREATE timeout even if the provider already registered media.

        The event and registry mutation share the lock used by _allocate.
        Otherwise a timeout can land *after* _allocate's cancellation check
        but *before* its registry insertion, leaving live RTP senders behind.
        Cleanup is best effort: only MediaLifecycle may certify remote STOP.
        """
        if cancel_requested is None:
            return
        with self._lock:
            cancel_requested.set()
            if cancel_identity is None:
                return
            operation_id, generation = cancel_identity
            session_id = self.recovery_session_id(operation_id, generation)
            entry = self._sessions.get(session_id)
            if entry is None or (entry.operation_id, entry.generation) != cancel_identity:
                return
            # Stop both outgoing tracks synchronously before scheduling peer
            # closure. No later SDP answer may re-open the revoked session.
            entry.closed = True
            entry.audio.stop()
            entry.video.stop()

        cleanup = asyncio.run_coroutine_threadsafe(
            self._stop(operation_id, session_id, generation), self._loop
        )

        def observe(future):
            try:
                future.result()
            except Exception:
                # Do not leak provider payloads or pretend this is a STOP receipt.
                LOG.warning("timed-out CREATE peer cleanup unconfirmed")

        cleanup.add_done_callback(observe)

    def _call(
        self,
        coroutine,
        cancel_requested: threading.Event | None = None,
        cancel_identity: tuple[str, int] | None = None,
        timeout: float | None = None,
    ):
        # Poll the owner cancellation event while the provider coroutine is
        # in flight. The lifecycle passes its remaining budget; this adapter
        # enforces it on the provider operation, not a detached waiter.
        budget = self._timeout if timeout is None else timeout
        if not 0 < budget <= 60:
            coroutine.close()
            raise ValueError("invalid WebRTC operation deadline")
        future = asyncio.run_coroutine_threadsafe(coroutine, self._loop)
        deadline = time.monotonic() + budget
        while True:
            if cancel_requested is not None and cancel_requested.is_set():
                self._fence_cancelled_create(cancel_requested, cancel_identity)
                future.cancel()
                raise RuntimeError("WebRTC operation cancelled; STOP unconfirmed")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                self._fence_cancelled_create(cancel_requested, cancel_identity)
                future.cancel()
                raise RuntimeError("WebRTC operation timed out; STOP unconfirmed")
            try:
                result = future.result(min(remaining, 0.05))
                if cancel_requested is not None and cancel_requested.is_set():
                    self._fence_cancelled_create(cancel_requested, cancel_identity)
                    future.cancel()
                    raise RuntimeError("WebRTC operation cancelled; STOP unconfirmed")
                if time.monotonic() >= deadline:
                    self._fence_cancelled_create(cancel_requested, cancel_identity)
                    future.cancel()
                    raise RuntimeError("WebRTC operation timed out; STOP unconfirmed")
                return result
            except concurrent.futures.TimeoutError:
                if future.done():
                    raise
            except concurrent.futures.CancelledError as exc:
                self._fence_cancelled_create(cancel_requested, cancel_identity)
                raise RuntimeError("WebRTC operation cancelled; STOP unconfirmed") from exc

    @staticmethod
    def recovery_session_id(operation_id: str, generation: int) -> str:
        """Exact preassigned local sender ID, not proof of allocation or STOP.

        This is safe only for this aiortc backend's deterministic identity.
        Providers assigning IDs after CREATE must not implement this contract.
        """
        return f"rt1-{operation_id}-{generation}"

    def allocate(self, operation_id: str, generation: int) -> str:
        return self.allocate_bounded(
            operation_id, generation, self._timeout, threading.Event()
        )

    def allocate_bounded(
        self,
        operation_id: str,
        generation: int,
        timeout: float,
        cancel_requested: threading.Event,
    ) -> str:
        """Allocate on the existing provider loop under caller deadline/revoke."""
        if cancel_requested.is_set():
            raise RuntimeError("WebRTC CREATE cancelled before dispatch")
        return self._call(
            self._allocate(operation_id, generation, cancel_requested),
            cancel_requested=cancel_requested,
            cancel_identity=(operation_id, generation),
            timeout=timeout,
        )

    async def _allocate(
        self, operation_id: str, generation: int,
        cancel_requested: threading.Event | None = None,
    ) -> str:
        if cancel_requested is not None and cancel_requested.is_set():
            raise asyncio.CancelledError("WebRTC CREATE cancelled before allocation")
        from aiortc import RTCPeerConnection, VideoStreamTrack, RTCConfiguration, RTCIceServer
        session_id = self.recovery_session_id(operation_id, generation)
        peer = RTCPeerConnection(configuration=RTCConfiguration(iceServers=[
            RTCIceServer(urls=item["urls"], username=item["username"], credential=item["credential"])
            for item in self._ice_servers
        ]))
        audio = _make_pcm_audio_track()
        video = None
        try:
            if self._video_renderer is None:
                # Diagnostic-only RTP: never a publishable Persona.
                video = VideoStreamTrack()
            else:
                # Publication readiness alone is not an allocation permit.
                # Re-probe before creating a sender to reject failed models.
                if not self.persona_video_ready():
                    raise RuntimeError("Persona video renderer unavailable")
                video = self._video_renderer.create_video_track(
                    operation_id, generation
                )
                # Reject a bare aiortc diagnostic source even when the
                # renderer incorrectly advertises itself as ready.
                if (not isinstance(video, VideoStreamTrack)
                        or type(video) is VideoStreamTrack):
                    raise RuntimeError("Persona renderer returned a diagnostic video track")
                # Real lip synchronization must consume the SAME authorized
                # PCM that is queued on this session's audio RTP sender.
                # The renderer method must enqueue bounded work and return
                # promptly; inference must never run under the sender lock.
                if not callable(getattr(video, "push_pcm", None)):
                    raise RuntimeError("Persona video track lacks PCM sync input")
                if not callable(getattr(video, "bind_audio_track", None)):
                    raise RuntimeError("Persona video track lacks audio synchronization")
                video.bind_audio_track(audio)
            peer.addTrack(audio)
            peer.addTrack(video)
            await peer.setLocalDescription(await peer.createOffer())
            with self._lock:
                if cancel_requested is not None and cancel_requested.is_set():
                    # Cancellation may be swallowed inside a provider await.
                    # Close both tracks and the peer in the exception handler;
                    # never publish a late sender into the active registry.
                    # The concurrent Future is already cancelled. Propagate task
                    # cancellation, not an unobserved RuntimeError that would
                    # surface as "Task exception was never retrieved".
                    raise asyncio.CancelledError("WebRTC CREATE completed after cancellation")
                if session_id in self._sessions:
                    raise RuntimeError("WebRTC session already allocated")
                # Never reuse a receipt from an earlier sender with this ID.
                self._completed_stops.pop(session_id, None)
                self._sessions[session_id] = _Session(
                    peer, audio, video, operation_id, generation
                )
            return session_id
        except BaseException:
            audio.stop()
            if video is not None:
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
        if entry.closed or description is None or entry.peer.connectionState == "closed":
            raise RuntimeError("WebRTC offer not available")
        return {"type": description.type, "sdp": description.sdp, "ice_servers": self._ice_servers}

    def answer(self, operation_id: str, session_id: str, generation: int, sdp: str):
        return self._call(self._answer(operation_id, session_id, generation, sdp))

    async def _answer(self, operation_id: str, session_id: str, generation: int, sdp: str):
        from aiortc import RTCSessionDescription
        with self._lock:
            entry = self._sessions.get(session_id)
        if not entry or (entry.operation_id, entry.generation) != (operation_id, generation):
            raise RuntimeError("unknown or revoked WebRTC session")
        # One final session-level media gate: answer and STOP must never
        # overlap across an await point. A late answer cannot reopen a
        # sender after STOP has begun or been confirmed.
        async with entry.gate:
            with self._lock:
                current = self._sessions.get(session_id)
            if current is not entry or entry.closed:
                raise RuntimeError("WebRTC operation revoked before answer")
            if entry.peer.remoteDescription is not None:
                raise RuntimeError("WebRTC answer already accepted")
            await entry.peer.setRemoteDescription(
                RTCSessionDescription(sdp=sdp, type="answer")
            )
            if entry.closed:
                raise RuntimeError("WebRTC operation revoked during answer")

    def push_pcm(
        self, operation_id: str, session_id: str, generation: int,
        pcm: bytes, *, final: bool = False,
    ):
        """Queue bounded authorized TTS PCM on the existing RTP sender.

        The canonical durable worker HTTP boundary owns revocation
        serialization. This method cannot allocate a session or resurrect STOP.
        """
        with self._lock:
            entry = self._sessions.get(session_id)
            if (entry is None or entry.closed or
                    (entry.operation_id, entry.generation) != (operation_id, generation)):
                raise RuntimeError("remote audio sender revoked")
            if self._video_renderer is not None:
                # Model inference releases the SAME PCM to audio only once
                # matching face frames are queued. Never play audio early.
                # Explicit end-of-utterance flush prevents truncating the
                # final sub-400ms authorized speech fragment.
                if final:
                    entry.video.push_pcm(pcm, final=True)
                else:
                    entry.video.push_pcm(pcm)
            else:
                # Legacy diagnostic RTP and test fixtures retain the original
                # direct audio path, but cannot authorize public Persona links.
                entry.audio.push_pcm(pcm)

    def failed_renderer_sessions(self) -> tuple[tuple[str, int], ...]:
        """Read-only health evidence for the ONE durable STOP reconciler.

        A GPU error is asynchronous to the accepted PCM HTTP request.
        Quarantine live sessions through MediaLifecycle.cancel(), never
        close RTP peers here or invent an audio/video STOP receipt.
        """
        if self._video_renderer is None:
            return ()
        with self._lock:
            sessions = tuple(self._sessions.values())
        renderer_failed = not self.persona_video_ready()
        return tuple(
            (entry.operation_id, entry.generation)
            for entry in sessions
            if not entry.closed and (
                renderer_failed or
                (callable(getattr(entry.video, "render_failed", None))
                 and entry.video.render_failed())
            )
        )

    def stop(self, operation_id: str, session_id: str, generation: int) -> StopReceipt:
        return self._call(self._stop(operation_id, session_id, generation))

    async def _stop(self, operation_id: str, session_id: str, generation: int) -> StopReceipt:
        with self._lock:
            entry = self._sessions.get(session_id)
            verified = self._completed_stops.get(session_id) if entry is None else None
        if entry is None and verified is not None and (
            verified.operation_id, verified.generation
        ) == (operation_id, generation):
            return verified
        if not entry or (entry.operation_id, entry.generation) != (operation_id, generation):
            # Never fabricate STOP for a session that may live on another worker.
            raise RuntimeError("remote WebRTC sender missing; STOP unconfirmed")
        # Set the session's irrevocable STOP intent before waiting for an
        # in-flight setRemoteDescription(). A hanging answer may hold the
        # session gate; it must not be able to report success after STOP began.
        entry.closed = True
        # Stop actual outgoing tracks before waiting for a stalled SDP answer.
        # The gate still serializes peer.close() with setRemoteDescription().
        # An unconfirmed close remains unconfirmed; these local track states
        # alone never authorize a durable STOP receipt.
        entry.audio.stop()
        entry.video.stop()
        # A generated frame may still be running on CUDA after track.stop().
        # Do not certify dual-track STOP until that worker has quiesced.
        wait_quiesced = getattr(entry.video, "wait_quiesced", None)
        if wait_quiesced is not None:
            if not await asyncio.to_thread(wait_quiesced, self._timeout):
                raise RuntimeError("Persona GPU video STOP unconfirmed")
        async with entry.gate:
            await entry.peer.close()
            if (
                entry.audio.readyState != "ended"
                or entry.video.readyState != "ended"
                or entry.peer.connectionState != "closed"
            ):
                raise RuntimeError("WebRTC sender shutdown not independently confirmed")
            receipt = StopReceipt(operation_id, session_id, generation, True, True)
            with self._lock:
                if self._sessions.get(session_id) is entry:
                    # Publish the receipt atomically with removal. A late
                    # reconciliation must never observe a missing sender
                    # before its verified STOP is available.
                    self._completed_stops[session_id] = receipt
                    self._sessions.pop(session_id)
            return receipt

    def shutdown(self):
        with self._lock:
            pending = list(self._sessions.values())
        for entry in pending:
            try:
                self.stop(entry.operation_id, self.recovery_session_id(entry.operation_id, entry.generation), entry.generation)
            except Exception:
                # Caller must inspect its durable ledger and retry reconciliation.
                pass
        if self._thread.is_alive():
            self._loop.call_soon_threadsafe(self._loop.stop)
            self._thread.join(timeout=self._timeout)
        if self._thread.is_alive():
            raise RuntimeError("aiortc provider event loop did not stop before deadline")
        # A stopped loop still owns selector/socket resources until close().
        # Make repeated worker start/stop cycles deterministic and avoid
        # ResourceWarning from the abandoned event-loop object.
        if not self._loop.is_closed():
            self._loop.close()
