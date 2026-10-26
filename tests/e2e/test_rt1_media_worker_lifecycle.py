"""Deterministic worker lifecycle races; no claim of actual RTP playback."""
import importlib.util
import tempfile
import threading
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "rt1_media_worker_lifecycle", ROOT / "scripts" / "rt1_media_worker_lifecycle.py"
)
mod = importlib.util.module_from_spec(SPEC)
import sys
sys.modules[SPEC.name] = mod
SPEC.loader.exec_module(mod)


class Sender:
    def __init__(self):
        self.created = 0
        self.stopped = 0
        self.started = threading.Event()
        self.release = threading.Event()
        self.wait_on_create = False
        self.partial = False

    def allocate(self, operation_id, generation):
        self.created += 1
        self.started.set()
        if self.wait_on_create:
            if not self.release.wait(2):
                raise TimeoutError("backend hung")
        return "session-a"

    def stop(self, operation_id, session_id, generation):
        self.stopped += 1
        return mod.StopReceipt(
            operation_id, session_id, generation, True, not self.partial
        )


class LifecycleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.db = Path(self.temp.name) / "media.sqlite"
        self.sender = Sender()
        self.worker = mod.MediaLifecycle(self.db, self.sender)

    def tearDown(self):
        self.temp.cleanup()

    def test_idempotent_create_and_strict_stop(self):
        self.assertEqual(self.worker.create("op-a", 1), "session-a")
        self.assertEqual(self.worker.create("op-a", 1), "session-a")
        self.assertEqual(self.sender.created, 1)
        receipt = self.worker.cancel("op-a", 1)
        self.assertTrue(receipt.audio_stopped and receipt.video_stopped)
        self.assertEqual(self.worker.status("op-a")[1], "stopped")
        with self.assertRaises(RuntimeError):
            self.worker.create("op-a", 1)

    def test_atomic_create_admission_across_worker_connections(self):
        # Two worker processes may share the durable ledger but not a Python lock.
        # Hold the first admission before INSERT and race a second connection.
        # Without BEGIN IMMEDIATE, both can observe an empty ledger and allocate.
        other = mod.MediaLifecycle(self.db, self.sender)
        first_admitting = threading.Event()
        release_first = threading.Event()
        second_entered = threading.Event()
        second_read = threading.Event()
        original_write = self.worker._write
        original_get = other._get

        def paused_write(operation_id, generation, phase, session_id):
            if phase == "creating":
                first_admitting.set()
                if not release_first.wait(3):
                    raise AssertionError("first admission never released")
            return original_write(operation_id, generation, phase, session_id)

        def observed_get(operation_id):
            if operation_id == "op-b":
                second_read.set()
            return original_get(operation_id)

        self.worker._write = paused_write
        other._get = observed_get
        results = []
        failures = []

        def attempt(worker, operation_id):
            try:
                results.append(worker.create(operation_id, 1))
            except BaseException as error:
                failures.append(error)

        first = threading.Thread(target=attempt, args=(self.worker, "op-a"))
        second = threading.Thread(
            target=lambda: (second_entered.set(), attempt(other, "op-b"))
        )
        try:
            first.start()
            self.assertTrue(first_admitting.wait(2))
            second.start()
            self.assertTrue(second_entered.wait(1))
            self.assertFalse(
                second_read.wait(0.2),
                "second worker read an uncommitted media admission",
            )
        finally:
            release_first.set()
            first.join(3)
            if second.ident is not None:
                second.join(3)
        self.assertFalse(first.is_alive())
        self.assertFalse(second.is_alive())
        self.assertEqual(results, ["session-a"])
        self.assertEqual(len(failures), 1)
        self.assertIsInstance(failures[0], RuntimeError)
        self.assertEqual(self.sender.created, 1)

    def test_cancel_during_remote_create_forces_compensating_stop(self):
        self.sender.wait_on_create = True
        errors = []
        task = threading.Thread(
            target=lambda: self._run_create(errors), daemon=True
        )
        task.start()
        self.assertTrue(self.sender.started.wait(1))
        self.assertIsNone(self.worker.cancel("op-a", 1))
        self.sender.release.set()
        task.join(2)
        self.assertFalse(task.is_alive())
        self.assertEqual(self.sender.stopped, 1)
        self.assertEqual(self.worker.status("op-a")[1], "stopped")
        self.assertEqual(len(errors), 1)

    def _run_create(self, errors):
        try:
            self.worker.create("op-a", 1)
        except RuntimeError as error:
            errors.append(error)

    def test_partial_stop_quarantines_restart_and_recovery(self):
        self.worker.create("op-a", 1)
        self.sender.partial = True
        with self.assertRaises(RuntimeError):
            self.worker.cancel("op-a", 1)
        self.assertEqual(self.worker.status("op-a")[1], "uncertain")
        restarted = mod.MediaLifecycle(self.db, self.sender)
        with self.assertRaises(RuntimeError):
            restarted.create("op-b", 1)
        self.sender.partial = False
        restarted.reconcile()
        self.assertEqual(restarted.status("op-a")[1], "stopped")
        self.assertEqual(restarted.create("op-b", 1), "session-a")

    def test_unknown_id_after_restart_must_not_reallocate(self):
        self.worker._write("op-a", 1, "uncertain", None)
        restarted = mod.MediaLifecycle(self.db, self.sender)
        restarted.reconcile()
        with self.assertRaises(RuntimeError):
            restarted.create("op-b", 1)
        self.assertEqual(self.sender.created, 0)


if __name__ == "__main__":
    unittest.main()
