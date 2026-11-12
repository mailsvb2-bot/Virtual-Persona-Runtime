"""Durable STOP retry: transient network loss must not allow media resurrection."""
import sqlite3
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

from rt1_media_worker_lifecycle import MediaLifecycle, StopReceipt
from rt1_media_worker_server import drain_shutdown, reconcile_until_shutdown


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

    def test_restart_reconciles_only_with_real_reachable_sender_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "media.db"
            sender = FlakySender()
            first = MediaLifecycle(path, sender)
            first.create("recoverable", 1)
            # Simulate a provider whose sender identity survives the HTTP
            # coordinator restart. Its first STOP loses acknowledgement.
            with self.assertRaises(OSError):
                first.cancel("recoverable", 1)

            restarted = MediaLifecycle(path, sender)
            restarted.quarantine_after_restart()
            self.assertEqual(restarted.status("recoverable")[1], "uncertain")
            restarted.reconcile()
            self.assertEqual(
                restarted.status("recoverable"), (1, "stopped", "flaky-session")
            )
            self.assertFalse(sender.active)
            self.assertEqual(sender.stops, 2)

    def test_graceful_shutdown_revokes_live_sender_and_persists_dual_stop(self):
        with tempfile.TemporaryDirectory() as directory:
            sender = FlakySender()
            # This case has a reliable synchronous STOP, unlike FlakySender's
            # first failed acknowledgement.
            sender.stops = 1
            lifecycle = MediaLifecycle(Path(directory) / "media.db", sender)
            self.assertEqual(lifecycle.create("live-worker", 1), "flaky-session")
            self.assertTrue(sender.active)
            self.assertTrue(drain_shutdown(lifecycle, timeout=2))
            self.assertFalse(sender.active)
            self.assertEqual(
                lifecycle.status_with_stop_confirmation("live-worker"),
                (1, "stopped", "flaky-session", True),
            )
            with self.assertRaisesRegex(RuntimeError, "shutting down"):
                lifecycle.create("replacement", 1)
            with self.assertRaisesRegex(RuntimeError, "shutting down"):
                lifecycle.create_bounded("replacement", 1, 0.1)
            self.assertEqual(sender.stops, 2)
            # A second drain must not issue duplicate provider STOP.
            self.assertTrue(drain_shutdown(lifecycle, timeout=0.1))
            self.assertEqual(sender.stops, 2)

    def test_graceful_shutdown_failed_stop_remains_durable_and_denies_admission(self):
        class UnreachableSender(FlakySender):
            def stop(self, operation_id, session_id, generation):
                self.stops += 1
                raise ConnectionError("remote STOP unconfirmed")

        with tempfile.TemporaryDirectory() as directory:
            sender = UnreachableSender()
            lifecycle = MediaLifecycle(Path(directory) / "media.db", sender)
            lifecycle.create("live-worker", 1)
            self.assertFalse(drain_shutdown(lifecycle, timeout=0.25))
            self.assertEqual(
                lifecycle.status_with_stop_confirmation("live-worker"),
                (1, "uncertain", "flaky-session", False),
            )
            with self.assertRaises(RuntimeError):
                lifecycle.create("replacement", 1)
            # A new process does not get to infer STOP from local cleanup.
            restarted = MediaLifecycle(Path(directory) / "media.db", sender)
            restarted.quarantine_after_restart()
            self.assertFalse(restarted.status_with_stop_confirmation("live-worker")[3])
            with self.assertRaises(RuntimeError):
                restarted.create("replacement", 1)

    def test_shutdown_during_create_blocks_late_admission_and_compensates_sender(self):
        class PendingSender(FlakySender):
            def __init__(self):
                super().__init__()
                self.entered = threading.Event()
                self.release = threading.Event()

            def allocate(self, operation_id, generation):
                self.entered.set()
                if not self.release.wait(3):
                    raise RuntimeError("fixture timeout")
                return super().allocate(operation_id, generation)

            def stop(self, operation_id, session_id, generation):
                self.stops += 1
                self.active = False
                return StopReceipt(operation_id, session_id, generation, True, True)

        with tempfile.TemporaryDirectory() as directory:
            sender = PendingSender()
            lifecycle = MediaLifecycle(Path(directory) / "media.db", sender)
            results = []
            worker = threading.Thread(target=lambda: results.append(
                self._capture_create(lifecycle, "pending")
            ))
            worker.start()
            try:
                self.assertTrue(sender.entered.wait(2))
                self.assertFalse(drain_shutdown(lifecycle, timeout=0.1))
                self.assertEqual(lifecycle.status("pending")[1], "uncertain")
                with self.assertRaisesRegex(RuntimeError, "shutting down"):
                    lifecycle.create("replacement", 1)
            finally:
                sender.release.set()
                worker.join(timeout=3)
            self.assertFalse(worker.is_alive())
            self.assertEqual(results, [False], "revoked CREATE must never be admitted")
            self.assertEqual(
                lifecycle.status_with_stop_confirmation("pending"),
                (1, "stopped", "flaky-session", True),
            )
            self.assertFalse(sender.active)

    @staticmethod
    def _capture_create(lifecycle, operation_id):
        try:
            lifecycle.create(operation_id, 1)
        except RuntimeError:
            return False
        return True

    def test_shutdown_waits_for_late_both_track_receipt_within_global_deadline(self):
        class LateSender(FlakySender):
            def stop(self, operation_id, session_id, generation):
                self.stops += 1
                time.sleep(1.25)
                self.active = False
                return StopReceipt(operation_id, session_id, generation, True, True)

        with tempfile.TemporaryDirectory() as directory:
            sender = LateSender()
            lifecycle = MediaLifecycle(Path(directory) / "media.db", sender)
            lifecycle.create("late-stop", 1)
            started = time.monotonic()
            self.assertTrue(drain_shutdown(lifecycle, timeout=2.5))
            self.assertGreaterEqual(time.monotonic() - started, 1.2)
            self.assertLess(time.monotonic() - started, 2.5)
            self.assertEqual(
                lifecycle.status_with_stop_confirmation("late-stop"),
                (1, "stopped", "flaky-session", True),
            )
            self.assertEqual(sender.stops, 1)

    def test_shutdown_quarantine_obeys_deadline_when_sqlite_writer_busy(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "media.db"
            sender = FlakySender()
            lifecycle = MediaLifecycle(path, sender)
            lifecycle.create("active", 1)
            competing = sqlite3.connect(path, isolation_level=None)
            try:
                competing.execute("BEGIN IMMEDIATE")
                started = time.monotonic()
                self.assertFalse(drain_shutdown(lifecycle, timeout=0.07))
                self.assertLess(time.monotonic() - started, 0.35)
                with self.assertRaisesRegex(RuntimeError, "shutting down"):
                    lifecycle.create("late", 1)
                self.assertEqual(lifecycle.status("active")[1], "active")
            finally:
                competing.execute("ROLLBACK")
                competing.close()
            # The fixture intentionally fails its FIRST STOP acknowledgement;
            # isolate database contention rather than testing STOP retry here.
            sender.stops = 1
            self.assertTrue(drain_shutdown(lifecycle, timeout=1.5))
            self.assertEqual(
                lifecycle.status_with_stop_confirmation("active"),
                (1, "stopped", "flaky-session", True),
            )

    def test_shutdown_lock_wait_obeys_deadline_when_inprocess_writer_busy(self):
        with tempfile.TemporaryDirectory() as directory:
            lifecycle = MediaLifecycle(Path(directory) / "media.db", FlakySender())
            started_lock = threading.Event()
            release_lock = threading.Event()

            def hold_lock():
                with lifecycle._lock:
                    started_lock.set()
                    release_lock.wait(2)

            worker = threading.Thread(target=hold_lock)
            worker.start()
            try:
                self.assertTrue(started_lock.wait(1))
                start = time.monotonic()
                self.assertFalse(drain_shutdown(lifecycle, timeout=0.06))
                self.assertLess(time.monotonic() - start, 0.3)
            finally:
                release_lock.set()
                worker.join(timeout=1)
            self.assertTrue(drain_shutdown(lifecycle, timeout=0.5))

    def test_reconciler_rejects_nonpositive_interval(self):
        with tempfile.TemporaryDirectory() as directory:
            lifecycle = MediaLifecycle(Path(directory) / "media.db", FlakySender())
            with self.assertRaises(ValueError):
                reconcile_until_shutdown(lifecycle, threading.Event(), 0)


if __name__ == "__main__":
    unittest.main()
