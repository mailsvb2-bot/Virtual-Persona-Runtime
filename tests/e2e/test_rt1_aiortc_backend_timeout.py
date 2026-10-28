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


if __name__ == "__main__":
    unittest.main()
