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
