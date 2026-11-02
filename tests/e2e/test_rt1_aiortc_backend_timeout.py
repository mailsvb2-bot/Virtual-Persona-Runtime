"""Timeouts must cancel pending aiortc work, never claim confirmed STOP."""
import asyncio
import sys
import threading
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from rt1_aiortc_backend import AiortcSenderBackend


class SenderTimeoutTests(unittest.TestCase):
    def test_preassigned_sender_identity_matches_allocation_contract(self):
        self.assertEqual(
            AiortcSenderBackend.recovery_session_id("op-42", 7),
            "rt1-op-42-7",
        )

    def test_successful_loop_call(self):
        backend = AiortcSenderBackend(timeout=2.0)
        try:
            self.assertEqual(backend._call(asyncio.sleep(0, result="ok")), "ok")
        finally:
            backend.shutdown()

    def test_cancelled_worker_future_does_not_confirm_stop(self):
        from concurrent.futures import Future
        from unittest.mock import patch

        backend = AiortcSenderBackend(timeout=0.2)
        cancelled = Future()
        cancelled.cancel()

        def fake_submit(coroutine, loop):
            coroutine.close()
            return cancelled

        try:
            with patch(
                "rt1_aiortc_backend.asyncio.run_coroutine_threadsafe",
                side_effect=fake_submit,
            ):
                with self.assertRaisesRegex(RuntimeError, "cancelled; STOP unconfirmed"):
                    backend._call(asyncio.sleep(0))
        finally:
            backend.shutdown()

    def test_timed_out_operation_is_cancelled_without_false_stop(self):
        backend = AiortcSenderBackend(timeout=0.2)
        started = threading.Event()
        cancelled = threading.Event()
        late_effect = threading.Event()

        async def pending_operation():
            started.set()
            try:
                await asyncio.sleep(1.0)
                late_effect.set()
            finally:
                cancelled.set()

        try:
            with self.assertRaisesRegex(RuntimeError, "STOP unconfirmed"):
                backend._call(pending_operation())
            self.assertTrue(started.is_set(), "test did not exercise an in-flight operation")
            self.assertTrue(cancelled.wait(2.0), "timed-out operation was not cancelled")
            self.assertFalse(late_effect.is_set(), "timed-out operation completed late")
        finally:
            backend.shutdown()


    def test_provider_swallows_cancellation_but_late_create_cannot_publish_sender(self):
        # Simulated non-cooperative aiortc provider, NOT real D-ID evidence.
        # asyncio Future.cancel() alone is insufficient: the provider may
        # swallow CancelledError and complete CREATE after the HTTP deadline.
        from types import SimpleNamespace
        from unittest.mock import patch

        started = threading.Event()
        closed = threading.Event()
        created = []

        class Track:
            def __init__(self):
                self.readyState = "live"

            def stop(self):
                self.readyState = "ended"

        class Peer:
            def __init__(self, configuration):
                self.audio = None
                self.video = None
                self.connectionState = "new"
                created.append(self)

            def addTrack(self, track):
                if self.audio is None:
                    self.audio = track
                else:
                    self.video = track

            async def createOffer(self):
                return object()

            async def setLocalDescription(self, description):
                started.set()
                try:
                    await asyncio.sleep(5)
                except asyncio.CancelledError:
                    # Deliberately ignore cancellation, as an uncooperative
                    # provider implementation may do during network teardown.
                    await asyncio.sleep(0.05)

            async def close(self):
                self.connectionState = "closed"
                closed.set()

        fake_aiortc = SimpleNamespace(
            AudioStreamTrack=Track,
            VideoStreamTrack=Track,
            RTCPeerConnection=Peer,
            RTCConfiguration=lambda **kwargs: kwargs,
            RTCIceServer=lambda **kwargs: kwargs,
        )
        backend = AiortcSenderBackend(timeout=0.2)
        try:
            with patch.dict(sys.modules, {"aiortc": fake_aiortc}):
                with self.assertRaisesRegex(RuntimeError, "timed out; STOP unconfirmed"):
                    backend.allocate("late-create", 1)
                self.assertTrue(started.is_set(), "CREATE did not reach the provider")
                self.assertTrue(closed.wait(2), "late CREATE failed to close its peer")
                self.assertEqual(len(created), 1)
                self.assertEqual(created[0].audio.readyState, "ended")
                self.assertEqual(created[0].video.readyState, "ended")
                self.assertEqual(backend._sessions, {}, "revoked CREATE published a sender")
        finally:
            backend.shutdown()


    def test_timeout_after_sender_registration_stops_both_tracks_and_peer(self):
        # Regression for timeout landing after the provider's last cancellation
        # check but before its CREATE future is delivered to the caller.
        # This is a simulated peer, not evidence of D-ID remote RTP STOP.
        from rt1_aiortc_backend import _Session

        registered = threading.Event()
        peer_closed = threading.Event()
        cancel_requested = threading.Event()

        class Track:
            def __init__(self):
                self.readyState = "live"

            def stop(self):
                self.readyState = "ended"

        class Peer:
            connectionState = "connected"

            async def close(self):
                self.connectionState = "closed"
                peer_closed.set()

        backend = AiortcSenderBackend(timeout=0.2)
        audio, video, peer = Track(), Track(), Peer()
        session_id = backend.recovery_session_id("late-registered", 1)

        async def register_then_stall():
            with backend._lock:
                backend._sessions[session_id] = _Session(
                    peer, audio, video, "late-registered", 1
                )
            registered.set()
            await asyncio.sleep(30)

        try:
            with self.assertRaisesRegex(RuntimeError, "timed out; STOP unconfirmed"):
                backend._call(
                    register_then_stall(),
                    cancel_requested=cancel_requested,
                    cancel_identity=("late-registered", 1),
                )
            self.assertTrue(registered.is_set())
            self.assertTrue(cancel_requested.is_set())
            self.assertEqual(audio.readyState, "ended")
            self.assertEqual(video.readyState, "ended")
            self.assertTrue(peer_closed.wait(2), "timed-out CREATE peer was not closed")
            self.assertNotIn(session_id, backend._sessions)
        finally:
            backend.shutdown()


    def test_pre_cancelled_create_never_calls_provider(self):
        # A queued CREATE whose deadline expired before scheduling must not
        # even import/construct the remote transport.
        cancel_requested = threading.Event()
        cancel_requested.set()
        backend = AiortcSenderBackend(timeout=0.2)
        try:
            with self.assertRaises(asyncio.CancelledError):
                asyncio.run(backend._allocate("never-start", 1, cancel_requested))
            self.assertEqual(backend._sessions, {})
        finally:
            backend.shutdown()



class SenderStopGateTests(unittest.IsolatedAsyncioTestCase):
    async def test_remote_answer_must_complete_before_confirmed_stop(self):
        from types import SimpleNamespace
        from unittest.mock import patch
        from rt1_aiortc_backend import _Session

        answer_started = asyncio.Event()
        release_answer = asyncio.Event()
        events = []

        class Track:
            readyState = "live"

            def stop(self):
                self.readyState = "ended"
                events.append("track-stop")

        class Peer:
            connectionState = "connected"
            # Supply a valid offer so the post-STOP denial cannot pass merely
            # because the fake peer has no SDP to offer.
            localDescription = SimpleNamespace(type="offer", sdp="v=0\r\n")
            remoteDescription = None

            async def setRemoteDescription(self, description):
                answer_started.set()
                await release_answer.wait()
                self.remoteDescription = description
                events.append("answer")

            async def close(self):
                self.connectionState = "closed"
                events.append("peer-stop")

        backend = AiortcSenderBackend(timeout=2)
        peer = Peer()
        session = _Session(peer, Track(), Track(), "op", 1)
        backend._sessions["session"] = session
        try:
            self.assertEqual(
                (await backend._offer("op", "session", 1))["type"], "offer",
                "test must start with an offerable, authorized session",
            )
            with patch.dict(
                sys.modules,
                {"aiortc": SimpleNamespace(RTCSessionDescription=lambda **values: values)},
            ):
                answer = asyncio.create_task(backend._answer("op", "session", 1, "sdp"))
                await asyncio.wait_for(answer_started.wait(), 1)
                stop = asyncio.create_task(backend._stop("op", "session", 1))
                await asyncio.sleep(0.01)
                self.assertFalse(stop.done(), "STOP raced unfinished SDP answer")
                self.assertTrue(session.closed, "STOP must fence the in-flight answer immediately")
                self.assertEqual(session.audio.readyState, "ended", "audio must stop before stalled SDP releases")
                self.assertEqual(session.video.readyState, "ended", "video must stop before stalled SDP releases")
                self.assertNotIn("peer-stop", events, "peer closure must await in-flight SDP")
                with self.assertRaisesRegex(RuntimeError, "offer not available"):
                    await backend._offer("op", "session", 1)
                release_answer.set()
                with self.assertRaisesRegex(RuntimeError, "revoked during answer"):
                    await asyncio.wait_for(answer, 1)
                receipt = await asyncio.wait_for(stop, 1)
                self.assertTrue(receipt.audio_stopped)
                self.assertTrue(receipt.video_stopped)
                self.assertLess(events.index("answer"), events.index("peer-stop"))
                with self.assertRaises(RuntimeError):
                    await backend._answer("op", "session", 1, "late-sdp")
        finally:
            backend.shutdown()

if __name__ == "__main__":
    unittest.main()
