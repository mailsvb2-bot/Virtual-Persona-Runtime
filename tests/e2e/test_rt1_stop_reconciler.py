"""Durable STOP retry: transient network loss must not allow media resurrection."""
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

from rt1_media_worker_lifecycle import MediaLifecycle, StopReceipt
from rt1_media_worker_server import reconcile_until_shutdown


class FlakySender:
    def __init__(self):
        self.stops = 0
        self.active = False

    def allocate(self, operation_id, generation):
        self.active = True
        return "flaky-session"

    def stop(self, operation_id, session_id, generation):
        self.stops += 1
        if self.stops == 1:
            raise OSError("simulated lost STOP acknowledgement")
        self.active = False
        return StopReceipt(operation_id, session_id, generation, True, True)


class ReconcileTests(unittest.TestCase):
    def test_network_failure_retains_fence_until_stop_is_confirmed(self):
        with tempfile.TemporaryDirectory() as directory:
            sender = FlakySender()
            lifecycle = MediaLifecycle(Path(directory) / "media.db", sender)
            self.assertEqual(lifecycle.create("op-reconcile", 1), "flaky-session")
            with self.assertRaises(OSError):
                lifecycle.cancel("op-reconcile", 1)
            self.assertEqual(lifecycle.status("op-reconcile")[1], "uncertain")
            self.assertTrue(sender.active)
            with self.assertRaises(RuntimeError):
                lifecycle.create("another-operation", 1)

            shutdown = threading.Event()
            runner = threading.Thread(
                target=reconcile_until_shutdown, args=(lifecycle, shutdown, 0.01)
            )
            runner.start()
            try:
                deadline = time.monotonic() + 2
                while time.monotonic() < deadline:
                    if lifecycle.status("op-reconcile")[1] == "stopped":
                        break
                    time.sleep(0.01)
                self.assertEqual(lifecycle.status("op-reconcile")[1], "stopped")
                self.assertFalse(sender.active)
                self.assertGreaterEqual(sender.stops, 2)
                with self.assertRaises(RuntimeError):
                    lifecycle.create("op-reconcile", 1)
            finally:
                shutdown.set()
                runner.join(timeout=1)
                self.assertFalse(runner.is_alive())

    def test_restart_cannot_invent_stop_for_lost_sender(self):
        # Simulate a process restart with the same durable SQLite ledger but
        # a fresh backend that has no previous sender handle. That backend
        # cannot authenticate remote shutdown and must not clear quarantine.
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "media.db"
            original = FlakySender()
            first = MediaLifecycle(path, original)
            self.assertEqual(first.create("orphan", 1), "flaky-session")
            with self.assertRaises(OSError):
                first.cancel("orphan", 1)
            self.assertEqual(first.status("orphan")[1], "uncertain")

            class RestartedSender:
                def allocate(self, operation_id, generation):
                    raise AssertionError("recovery may not create a replacement")

                def stop(self, operation_id, session_id, generation):
                    raise RuntimeError("sender handle lost; STOP unconfirmed")

            restarted = MediaLifecycle(path, RestartedSender())
            with self.assertRaisesRegex(RuntimeError, "STOP unconfirmed"):
                restarted.reconcile()
            self.assertEqual(restarted.status("orphan"), (1, "uncertain", "flaky-session"))
            with self.assertRaises(RuntimeError):
                restarted.create("replacement", 1)
            with self.assertRaises(RuntimeError):
                restarted.create("orphan", 1)
            self.assertTrue(original.active, "test must not counterfeit remote teardown")

    def test_restart_quarantines_active_id_before_any_http_admission(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "media.db"
            original = FlakySender()
            first = MediaLifecycle(path, original)
            self.assertEqual(first.create("old-active", 1), "flaky-session")
            self.assertEqual(first.status("old-active")[1], "active")

            class FreshBackend:
                def allocate(self, operation_id, generation):
                    raise AssertionError("stale CREATE must not reach provider")

                def stop(self, operation_id, session_id, generation):
                    raise RuntimeError("old sender unavailable; STOP unconfirmed")

            restarted = MediaLifecycle(path, FreshBackend())
            restarted.quarantine_after_restart()
            self.assertEqual(
                restarted.status("old-active"), (1, "uncertain", "flaky-session")
            )
            with self.assertRaises(RuntimeError):
                restarted.create("old-active", 1)
            with self.assertRaises(RuntimeError):
                restarted.create("new-visitor", 1)
            with self.assertRaisesRegex(RuntimeError, "STOP unconfirmed"):
                restarted.reconcile()
            self.assertEqual(restarted.status("old-active")[1], "uncertain")
            self.assertTrue(original.active)

    def test_reconciler_rejects_nonpositive_interval(self):
        with tempfile.TemporaryDirectory() as directory:
            lifecycle = MediaLifecycle(Path(directory) / "media.db", FlakySender())
            with self.assertRaises(ValueError):
                reconcile_until_shutdown(lifecycle, threading.Event(), 0)


if __name__ == "__main__":
    unittest.main()
