#!/usr/bin/env python3
"""Test-only synthetic Persona renderer for the RT1 browser journey.

This fixture uses the real aiortc sender, durable worker ledger, authenticated
HTTP boundary and dual-track STOP implementation. Its generated face is not a
production renderer and is not evidence of a live D-ID session.
"""
from __future__ import annotations

import os
import signal
import tempfile
import threading
from http.server import ThreadingHTTPServer
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import aiortc
import numpy as np
from av import VideoFrame

from rt1_aiortc_backend import AiortcSenderBackend
from rt1_media_worker_http import handler_for
from rt1_media_worker_lifecycle import MediaLifecycle
from rt1_media_worker_server import drain_shutdown, reconcile_until_shutdown


class SyntheticPersonaVideoTrack(aiortc.VideoStreamTrack):
    total_frames = 0
    total_frames_lock = threading.Lock()

    def __init__(self):
        super().__init__()
        self.frames_sent = 0
        y, x = np.mgrid[0:240, 0:320]
        pixels = np.full((240, 320, 3), (22, 30, 44), dtype=np.uint8)
        face = ((x - 160) ** 2) / (72 ** 2) + ((y - 120) ** 2) / (90 ** 2) <= 1
        pixels[face] = (190, 152, 126)
        left_eye = ((x - 135) ** 2) / (8 ** 2) + ((y - 105) ** 2) / (5 ** 2) <= 1
        right_eye = ((x - 185) ** 2) / (8 ** 2) + ((y - 105) ** 2) / (5 ** 2) <= 1
        pixels[left_eye | right_eye] = (24, 24, 28)
        mouth = (y >= 151) & (y <= 157) & (x >= 139) & (x <= 181)
        pixels[mouth] = (100, 34, 42)
        self._pixels = pixels

    async def recv(self):
        pts, time_base = await self.next_timestamp()
        frame = VideoFrame.from_ndarray(self._pixels, format="rgb24")
        frame.pts = pts
        frame.time_base = time_base
        self.frames_sent += 1
        with type(self).total_frames_lock:
            type(self).total_frames += 1
        return frame


# AiortcSenderBackend imports VideoStreamTrack at allocation time. Restrict the
# replacement to this test process; production worker code remains fail-closed.
aiortc.VideoStreamTrack = SyntheticPersonaVideoTrack


class SyntheticPersonaSenderBackend(AiortcSenderBackend):
    def __init__(self):
        self._fixture_guard = threading.Lock()
        self._pcm_pushes = 0
        super().__init__()

    def persona_video_ready(self) -> bool:
        return True

    def push_pcm(self, operation_id: str, session_id: str, generation: int, pcm: bytes):
        super().push_pcm(operation_id, session_id, generation, pcm)
        with self._fixture_guard:
            self._pcm_pushes += 1

    def fixture_state(self) -> dict:
        with self._lock:
            sessions = tuple(self._sessions.values())
            receipts = tuple(self._completed_stops.values())
        with self._fixture_guard:
            pcm_pushes = self._pcm_pushes
        with SyntheticPersonaVideoTrack.total_frames_lock:
            video_frames_sent = SyntheticPersonaVideoTrack.total_frames
        return {
            "renderer": "synthetic-generated-face-test-only-not-did",
            "video_frames_sent": video_frames_sent,
            "pcm_pushes": pcm_pushes,
            "active_operations": len(sessions),
            "audio_senders_live": sum(entry.audio.readyState == "live" for entry in sessions),
            "video_senders_live": sum(entry.video.readyState == "live" for entry in sessions),
            "confirmed_stops": [
                {
                    "operation_id": receipt.operation_id,
                    "session_id": receipt.session_id,
                    "audio_stopped": receipt.audio_stopped,
                    "video_stopped": receipt.video_stopped,
                }
                for receipt in receipts
            ],
        }


def main() -> None:
    os.environ["RT1_WORKER_TOKEN"] = "rt1-local-e2e-secret"
    os.environ["RT1_WORKER_PORT"] = "18789"
    os.environ["RT1_WORKER_BODY_TIMEOUT_SECONDS"] = "5"
    database = Path(tempfile.mkdtemp(prefix="rt1-persona-worker-e2e-")) / "worker.sqlite3"
    os.environ["RT1_WORKER_DB"] = str(database)

    backend = SyntheticPersonaSenderBackend()
    lifecycle = MediaLifecycle(str(database), backend)
    lifecycle.quarantine_after_restart()
    base_handler = handler_for(lifecycle, os.environ["RT1_WORKER_TOKEN"], body_timeout_seconds=5.0)

    class TestHandler(base_handler):
        def do_GET(self):
            if self.path == "/__state":
                if not self._authorized():
                    self._send(401, {"error": "unauthorized"})
                else:
                    self._send(200, backend.fixture_state())
                return
            super().do_GET()

    server = ThreadingHTTPServer(("127.0.0.1", 18789), TestHandler)
    server.daemon_threads = True
    shutdown = threading.Event()
    reconciler = threading.Thread(
        target=reconcile_until_shutdown,
        args=(lifecycle, shutdown, 0.5),
        name="rt1-test-stop-reconcile",
        daemon=True,
    )
    reconciler.start()

    def request_shutdown(_signum, _frame):
        lifecycle.request_shutdown()
        threading.Thread(target=server.shutdown, name="rt1-test-http-shutdown", daemon=True).start()

    previous_term = signal.signal(signal.SIGTERM, request_shutdown)
    try:
        server.serve_forever(poll_interval=0.2)
    finally:
        lifecycle.request_shutdown()
        server.server_close()
        shutdown.set()
        reconciler.join(timeout=3)
        confirmed = drain_shutdown(lifecycle, timeout=4.0)
        backend.shutdown()
        signal.signal(signal.SIGTERM, previous_term)
        if not confirmed:
            raise RuntimeError("test worker shutdown could not confirm both RTP senders")


if __name__ == "__main__":
    main()
