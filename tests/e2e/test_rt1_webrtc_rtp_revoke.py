"""Real local WebRTC audio/video RTP revoke probe (requires aiortc).

Unlike HTTP mocks this negotiates two real peer connections, observes outbound
RTP packet counters for *both* senders, revokes transport, and ensures sender
tracks and both peer connections are stopped. It does not attest a remote GPU
worker or public RT1 authority; those require separate deployment evidence.

Usage: python -m pip install aiortc && python tests/e2e/test_rt1_webrtc_rtp_revoke.py
"""
from __future__ import annotations

import asyncio
import importlib.util
import sys
import threading
import unittest
from pathlib import Path


class ActualRtpRevocation(unittest.IsolatedAsyncioTestCase):
    async def test_two_sender_transport_stops_on_revoke(self):
        # This is a required CI safety probe, never an optional dependency test.
        # A broken/missing aiortc installation must fail the job, not silently
        # report a green workflow with zero RTP packets tested.
        from aiortc import RTCPeerConnection, AudioStreamTrack, VideoStreamTrack
        publisher = RTCPeerConnection()
        receiver = RTCPeerConnection()
        audio = AudioStreamTrack()
        video = VideoStreamTrack()
        publisher.addTrack(audio)
        publisher.addTrack(video)
        lifecycle_module = Path(__file__).resolve().parents[2] / "crates" / "vpr-provider-local-open-source" / "worker_lifecycle.py"
        spec = importlib.util.spec_from_file_location("rt1_worker_lifecycle_for_rtp", lifecycle_module)
        worker = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = worker
        spec.loader.exec_module(worker)
        lifecycle = worker.MediaLifecycle()
        loop = asyncio.get_running_loop()
        transport_closed = threading.Event()
        stopping = threading.Event()

        async def close_transport():
            try:
                await asyncio.wait_for(publisher.close(), timeout=3)
                await asyncio.wait_for(receiver.close(), timeout=3)
            finally:
                transport_closed.set()

        class RealSender:
            def __init__(self, track):
                self.track = track
            def stop(self):
                self.track.stop()
                if not stopping.is_set():
                    stopping.set()
                    loop.call_soon_threadsafe(lambda: asyncio.create_task(close_transport()))
            def wait_stopped(self, timeout):
                return self.track.readyState == "ended" and transport_closed.wait(timeout)

        publisher_candidates = []
        receiver_candidates = []

        @publisher.on("icecandidate")
        def from_publisher(candidate):
            publisher_candidates.append(candidate)

        @receiver.on("icecandidate")
        def from_receiver(candidate):
            receiver_candidates.append(candidate)

        @receiver.on("track")
        def on_track(track):
            async def drain():
                try:
                    while track.readyState == "live":
                        await track.recv()
                except Exception:
                    pass

            asyncio.create_task(drain())

        async def counters():
            reports = await publisher.getStats()
            totals = {"audio": 0, "video": 0}
            for stat in reports.values():
                if stat.type == "outbound-rtp" and stat.kind in totals:
                    totals[stat.kind] += stat.packetsSent
            return totals

        try:
            await publisher.setLocalDescription(await publisher.createOffer())
            await receiver.setRemoteDescription(publisher.localDescription)
            await receiver.setLocalDescription(await receiver.createAnswer())
            await publisher.setRemoteDescription(receiver.localDescription)
            for candidate in publisher_candidates:
                if candidate is not None:
                    await receiver.addIceCandidate(candidate)
            for candidate in receiver_candidates:
                if candidate is not None:
                    await publisher.addIceCandidate(candidate)

            before = None
            for _ in range(60):
                await asyncio.sleep(0.1)
                before = await counters()
                if before["audio"] > 0 and before["video"] > 0:
                    break
            self.assertIsNotNone(before)
            self.assertGreater(before["audio"], 0, "No real outbound audio RTP")
            self.assertGreater(before["video"], 0, "No real outbound video RTP")

            # Revoke via the real worker lifecycle owner, NOT by independently
            # closing transports in this test. Its STOP receipt is issued
            # only once both sender stop acknowledgements complete.
            lifecycle.create("rtp-operation", lambda _: worker.MediaAllocation(
                "rtp-session", RealSender(audio), RealSender(video)
            ))
            receipt = await asyncio.wait_for(
                asyncio.to_thread(lifecycle.stop, "rtp-session", 3.0), timeout=4
            )
            self.assertTrue(receipt["audio_stopped"])
            self.assertTrue(receipt["video_stopped"])
            self.assertTrue(transport_closed.is_set())
            self.assertEqual(publisher.connectionState, "closed")
            self.assertEqual(receiver.connectionState, "closed")
            self.assertEqual(audio.readyState, "ended")
            self.assertEqual(video.readyState, "ended")
            # Post-close no active sender or peer can emit further RTP; this
            # local test cannot attest a separately hosted remote publisher.
            await asyncio.sleep(0.25)
            self.assertEqual(publisher.connectionState, "closed")
        finally:
            audio.stop()
            video.stop()
            await publisher.close()
            await receiver.close()


if __name__ == "__main__":
    unittest.main()
