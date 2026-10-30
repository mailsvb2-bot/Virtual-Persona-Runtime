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
    def test_successful_loop_call(self):
        backend = AiortcSenderBackend(timeout=2.0)
        try:
            self.assertEqual(backend._call(asyncio.sleep(0, result="ok")), "ok")
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
