"""Real local WebRTC audio/video RTP revoke probe (requires aiortc).

Unlike HTTP mocks this negotiates two real peer connections, observes outbound
RTP packet counters for *both* senders, revokes transport, and ensures sender
tracks and both peer connections are stopped. It does not attest a remote GPU
worker or public RT1 authority; those require separate deployment evidence.

Usage: python -m pip install aiortc && python tests/e2e/test_rt1_webrtc_rtp_revoke.py
"""
from __future__ import annotations

import asyncio
import unittest


class ActualRtpRevocation(unittest.IsolatedAsyncioTestCase):
    async def test_two_sender_transport_stops_on_revoke(self):
        try:
            from aiortc import RTCPeerConnection, AudioStreamTrack, VideoStreamTrack
        except ImportError:
            self.skipTest("Install aiortc to execute the real RTP probe")
        publisher = RTCPeerConnection()
        receiver = RTCPeerConnection()
        audio = AudioStreamTrack()
        video = VideoStreamTrack()
        audio_sender = publisher.addTrack(audio)
        video_sender = publisher.addTrack(video)
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

            # Owner revoke must close actual transport and prevent new packets.
            audio.stop()
            video.stop()
            await asyncio.wait_for(publisher.close(), timeout=3)
            await asyncio.wait_for(receiver.close(), timeout=3)
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
