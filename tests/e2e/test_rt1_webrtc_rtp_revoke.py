"""Real local RTP acceptance through the *ported* canonical RT1 worker.

Exercise durable MediaLifecycle -> AiortcSenderBackend -> actual browser-side
peer (aiortc receiver) -> durable cancel -> verified dual-track STOP.
This does not prove third-party D-ID remote media termination.
"""
from __future__ import annotations

import asyncio
import sys
from array import array
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

from rt1_aiortc_backend import AiortcSenderBackend
from rt1_media_worker_lifecycle import MediaLifecycle


class ActualRtpRevocation(unittest.IsolatedAsyncioTestCase):
    async def test_two_sender_transport_stops_on_revoke(self):
        # Required CI test: missing aiortc fails, never silently skips.
        from aiortc import RTCSessionDescription, RTCPeerConnection

        receiver = RTCPeerConnection()
        backend = AiortcSenderBackend(timeout=8)
        received = {"audio": 0, "video": 0}
        received_pcm = {"audible": False}
        drain_tasks = []

        @receiver.on("track")
        def on_track(track):
            async def drain():
                try:
                    while track.readyState == "live":
                        frame = await track.recv()
                        received[track.kind] += 1
                        if track.kind == "audio" and frame.to_ndarray().any():
                            received_pcm["audible"] = True
                except Exception:
                    # Closing a real transport terminates pending recv().
                    pass
            drain_tasks.append(asyncio.create_task(drain()))

        try:
            with tempfile.TemporaryDirectory() as directory:
                lifecycle = MediaLifecycle(Path(directory) / "rt1-media.sqlite", backend)
                operation, generation = "real-rtp", 1
                session_id = await asyncio.to_thread(lifecycle.create, operation, generation)
                self.assertEqual(
                    session_id, backend.recovery_session_id(operation, generation)
                )
                offer = await asyncio.to_thread(backend.offer, operation, session_id, generation)
                await receiver.setRemoteDescription(
                    RTCSessionDescription(sdp=offer["sdp"], type=offer["type"])
                )
                answer = await receiver.createAnswer()
                await receiver.setLocalDescription(answer)
                await asyncio.to_thread(
                    backend.answer, operation, session_id, generation,
                    receiver.localDescription.sdp,
                )

                # A real remote peer must decode both audio and video frames.
                for _ in range(100):
                    if received["audio"] > 0 and received["video"] > 0:
                        break
                    await asyncio.sleep(0.1)
                self.assertGreater(received["audio"], 0, "receiver decoded no audio RTP")
                self.assertGreater(received["video"], 0, "receiver decoded no video RTP")

                # Inject real nonzero PCM into the SAME canonical sender used
                # by the browser peer. Silence-only RTP must not pass this test.
                one_frame = array("h", [11000 if sample % 16 < 8 else -11000
                                        for sample in range(320)]).tobytes()
                await asyncio.to_thread(
                    backend.push_pcm, operation, session_id, generation,
                    one_frame * 25,
                )
                for _ in range(100):
                    if received_pcm["audible"]:
                        break
                    await asyncio.sleep(0.02)
                self.assertTrue(
                    received_pcm["audible"],
                    "browser never decoded the injected nonzero PCM RTP",
                )

                receipt = await asyncio.wait_for(
                    asyncio.to_thread(lifecycle.cancel, operation, generation),
                    timeout=10,
                )
                self.assertIsNotNone(receipt, "STOP receipt must not be a tombstone")
                self.assertEqual(receipt.operation_id, operation)
                self.assertEqual(receipt.session_id, session_id)
                self.assertEqual(receipt.generation, generation)
                self.assertTrue(receipt.audio_stopped)
                self.assertTrue(receipt.video_stopped)
                # Backend returns STOP only after both real tracks and
                # the sending RTCPeerConnection are closed.
                self.assertNotIn(session_id, backend._sessions)
                self.assertIn(session_id, backend._completed_stops)
                with self.assertRaises(RuntimeError):
                    await asyncio.to_thread(
                        backend.push_pcm, operation, session_id, generation, one_frame
                    )
                with self.assertRaises(RuntimeError):
                    await asyncio.to_thread(lifecycle.create, operation, generation)

                frozen = dict(received)
                await asyncio.sleep(0.4)
                self.assertEqual(received, frozen, "RTP continued after verified STOP")
        finally:
            await receiver.close()
            for task in drain_tasks:
                task.cancel()
            if drain_tasks:
                await asyncio.gather(*drain_tasks, return_exceptions=True)
            backend.shutdown()


if __name__ == "__main__":
    unittest.main()
