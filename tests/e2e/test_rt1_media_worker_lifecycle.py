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

    def test_competing_stop_cannot_overwrite_confirmed_shutdown(self):
        # Independent worker connections must not issue concurrent remote STOPs.
        # A late failed attempt must never turn a verified 'stopped' into
        # 'uncertain', and a repeat CANCEL must not send STOP again.
        other = mod.MediaLifecycle(self.db, self.sender)
        entered = threading.Event()
        release = threading.Event()
        original_stop = self.sender.stop
        errors = []
        receipts = []

        def slow_stop(operation_id, session_id, generation):
            entered.set()
            if not release.wait(3):
                raise TimeoutError("test STOP stalled")
            return original_stop(operation_id, session_id, generation)

        self.sender.stop = slow_stop
        self.worker.create("op-a", 1)

        def first_cancel():
            try:
                receipts.append(self.worker.cancel("op-a", 1))
            except BaseException as error:
                errors.append(error)

        first = threading.Thread(target=first_cancel)
        first.start()
        try:
            self.assertTrue(entered.wait(2))
            with self.assertRaisesRegex(RuntimeError, "already in progress"):
                other.cancel("op-a", 1)
            self.assertEqual(other.status("op-a")[1], "stopping")
        finally:
            release.set()
            first.join(3)
        self.assertFalse(first.is_alive())
        self.assertEqual(errors, [])
        self.assertEqual(len(receipts), 1)
        self.assertTrue(receipts[0].audio_stopped and receipts[0].video_stopped)
        self.assertEqual(other.status("op-a")[1], "stopped")
        self.assertEqual(self.sender.stopped, 1)
        self.assertTrue(other.cancel("op-a", 1).video_stopped)
        self.assertEqual(self.sender.stopped, 1)

    def test_stop_lock_survives_other_process_and_recovers_after_exit(self):
        # A different process owns the STOP lock. CANCEL must fail closed
        # without sending a competing STOP; a subsequent worker can reconcile
        # once the OS releases the lock after process termination.
        import os
        import subprocess
        import sys

        if os.name == "nt":
            self.skipTest("POSIX interprocess flock regression")
        self.worker.create("op-a", 1)
        lock_path = str(self.db) + ".stop.lock"
        child = subprocess.Popen(
            [
                sys.executable,
                "-c",
                (
                    "import fcntl,os,sys;"
                    "fd=os.open(sys.argv[1],os.O_CREAT|os.O_RDWR,0o600);"
                    "fcntl.flock(fd,fcntl.LOCK_EX);"
                    "print('locked',flush=True);"
                    "sys.stdin.read(1)"
                ),
                lock_path,
            ],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
        )
        try:
            self.assertEqual(child.stdout.readline().strip(), b"locked")
            with self.assertRaisesRegex(RuntimeError, "already in progress"):
                self.worker.cancel("op-a", 1)
            self.assertEqual(self.worker.status("op-a")[1], "stopping")
            self.assertEqual(self.sender.stopped, 0)
        finally:
            child.kill()
            child.wait(timeout=3)
            child.stdout.close()
            child.stdin.close()
        restarted = mod.MediaLifecycle(self.db, self.sender)
        restarted.reconcile()
        self.assertEqual(restarted.status("op-a")[1], "stopped")
        self.assertEqual(self.sender.stopped, 1)

    def test_failed_stop_can_be_reconciled_by_restarted_worker(self):
        self.worker.create("op-a", 1)
        original_stop = self.sender.stop
        attempts = []

        def fail_once(operation_id, session_id, generation):
            attempts.append(1)
            if len(attempts) == 1:
                raise ConnectionError("STOP receipt lost")
            return original_stop(operation_id, session_id, generation)

        self.sender.stop = fail_once
        with self.assertRaises(ConnectionError):
            self.worker.cancel("op-a", 1)
        self.assertEqual(self.worker.status("op-a")[1], "uncertain")
        restarted = mod.MediaLifecycle(self.db, self.sender)
        restarted.reconcile()
        self.assertEqual(restarted.status("op-a")[1], "stopped")
        self.assertEqual(len(attempts), 2)

    def test_cancel_before_create_persists_tombstone_across_restart(self):
        other = mod.MediaLifecycle(self.db, self.sender)
        self.assertIsNone(other.cancel("op-a", 1))
        self.assertIsNone(other.cancel("op-a", 1))
        self.assertEqual(other.status("op-a")[1], "stopped")
        with self.assertRaisesRegex(RuntimeError, "cancelled"):
            self.worker.create("op-a", 1)
        restarted = mod.MediaLifecycle(self.db, self.sender)
        with self.assertRaisesRegex(RuntimeError, "cancelled"):
            restarted.create("op-a", 1)
        self.assertEqual(self.sender.created, 0)
        with self.assertRaises(KeyError):
            restarted.cancel("op-a", 2)
        self.assertEqual(restarted.status("op-a")[0], 1)

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
