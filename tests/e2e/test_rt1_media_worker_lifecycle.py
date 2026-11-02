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

    def test_cross_process_cancel_cannot_be_lost_after_create_reads_stale_phase(self):
        # The two worker objects have independent Python locks and SQLite
        # connections. Pause CREATE *after* it reads 'creating' on completion.
        # Without an IMMEDIATE transaction around read+write, the other worker
        # can commit CANCEL and CREATE can then overwrite it with 'active'.
        other = mod.MediaLifecycle(self.db, self.sender)
        read_creating = threading.Event()
        release_finalize = threading.Event()
        cancel_started = threading.Event()
        cancel_finished = threading.Event()
        original_get = self.worker._get
        created = []
        receipts = []
        failures = []

        def paused_get(operation_id):
            row = original_get(operation_id)
            if (
                operation_id == "op-a"
                and self.sender.created == 1
                and row is not None
                and row[1] == "creating"
            ):
                read_creating.set()
                if not release_finalize.wait(3):
                    raise AssertionError("CREATE finalization never released")
            return row

        self.worker._get = paused_get

        def run_create():
            try:
                created.append(self.worker.create("op-a", 1))
            except BaseException as error:
                failures.append(error)

        def run_cancel():
            cancel_started.set()
            try:
                receipts.append(other.cancel("op-a", 1))
            except BaseException as error:
                failures.append(error)
            finally:
                cancel_finished.set()

        creator = threading.Thread(target=run_create)
        canceller = threading.Thread(target=run_cancel)
        try:
            creator.start()
            self.assertTrue(read_creating.wait(2))
            canceller.start()
            self.assertTrue(cancel_started.wait(1))
            self.assertFalse(
                cancel_finished.wait(0.2),
                "CANCEL committed between CREATE completion read and write",
            )
        finally:
            release_finalize.set()
            creator.join(3)
            if canceller.ident is not None:
                canceller.join(3)
        self.assertFalse(creator.is_alive())
        self.assertFalse(canceller.is_alive())
        self.assertEqual(failures, [])
        self.assertEqual(created, ["session-a"])
        self.assertEqual(len(receipts), 1)
        self.assertIsNotNone(receipts[0])
        self.assertTrue(receipts[0].audio_stopped and receipts[0].video_stopped)
        self.assertEqual(self.sender.created, 1)
        self.assertEqual(self.sender.stopped, 1)
        self.assertEqual(other.status("op-a")[1], "stopped")

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

    def test_lost_create_reply_with_known_sender_id_is_recoverable_after_restart(self):
        # The remote allocation may succeed before the CREATE response is lost.
        # A stable preassigned identity lets restart retry authenticated STOP,
        # but it must never turn the timeout itself into a STOP receipt.
        class LostReplySender(Sender):
            remote_active = False

            @staticmethod
            def recovery_session_id(operation_id, generation):
                return f"rt1-{operation_id}-{generation}"

            def allocate(self, operation_id, generation):
                self.created += 1
                self.remote_active = True
                raise TimeoutError("simulated lost CREATE reply")

            def stop(self, operation_id, session_id, generation):
                if not self.remote_active or session_id != self.recovery_session_id(operation_id, generation):
                    raise RuntimeError("sender STOP not confirmed")
                self.stopped += 1
                self.remote_active = False
                return mod.StopReceipt(operation_id, session_id, generation, True, True)

        sender = LostReplySender()
        worker = mod.MediaLifecycle(self.db, sender)
        with self.assertRaisesRegex(TimeoutError, "lost CREATE reply"):
            worker.create("op-lost", 1)
        # The known remote sender is compensated immediately, not left live
        # until a process restart or the periodic reconciler wakes up.
        self.assertEqual(worker.status("op-lost"), (1, "stopped", "rt1-op-lost-1"))
        self.assertFalse(sender.remote_active)
        self.assertEqual(sender.stopped, 1)
        restarted = mod.MediaLifecycle(self.db, sender)
        restarted.quarantine_after_restart()
        restarted.reconcile()
        self.assertEqual(restarted.status("op-lost"), (1, "stopped", "rt1-op-lost-1"))
        self.assertEqual(sender.stopped, 1)
        with self.assertRaises(RuntimeError):
            restarted.create("op-lost", 1)

    def test_lost_create_reply_failed_immediate_stop_remains_durable_and_retryable(self):
        # The first compensating STOP fails. No false receipt, no replacement
        # CREATE; a restarted worker retries the SAME known sender identity.
        class FlakyLostReplySender(Sender):
            fail_stop = True
            remote_active = False

            @staticmethod
            def recovery_session_id(operation_id, generation):
                return f"rt1-{operation_id}-{generation}"

            def allocate(self, operation_id, generation):
                self.created += 1
                self.remote_active = True
                raise TimeoutError("CREATE response lost after allocation")

            def stop(self, operation_id, session_id, generation):
                self.stopped += 1
                if self.fail_stop:
                    raise TimeoutError("STOP response unavailable")
                if not self.remote_active or session_id != self.recovery_session_id(operation_id, generation):
                    raise RuntimeError("wrong STOP target")
                self.remote_active = False
                return mod.StopReceipt(operation_id, session_id, generation, True, True)

        sender = FlakyLostReplySender()
        worker = mod.MediaLifecycle(self.db, sender)
        with self.assertRaisesRegex(TimeoutError, "CREATE response lost"):
            worker.create("op-lost-retry", 1)
        self.assertEqual(sender.stopped, 1)
        self.assertEqual(
            worker.status("op-lost-retry"),
            (1, "uncertain", "rt1-op-lost-retry-1"),
        )
        self.assertTrue(sender.remote_active)
        with self.assertRaises(RuntimeError):
            worker.create("replacement", 1)
        sender.fail_stop = False
        restarted = mod.MediaLifecycle(self.db, sender)
        restarted.quarantine_after_restart()
        restarted.reconcile()
        self.assertEqual(sender.stopped, 2)
        self.assertFalse(sender.remote_active)
        self.assertEqual(
            restarted.status("op-lost-retry"),
            (1, "stopped", "rt1-op-lost-retry-1"),
        )

    def test_lost_create_reply_without_authoritative_id_stays_quarantined(self):
        class UnknownIdSender(Sender):
            def allocate(self, operation_id, generation):
                self.created += 1
                raise TimeoutError("remote CREATE outcome unknown")

        sender = UnknownIdSender()
        worker = mod.MediaLifecycle(self.db, sender)
        with self.assertRaises(TimeoutError):
            worker.create("unknown", 1)
        self.assertEqual(worker.status("unknown"), (1, "uncertain", None))
        restarted = mod.MediaLifecycle(self.db, sender)
        restarted.quarantine_after_restart()
        restarted.reconcile()
        self.assertEqual(restarted.status("unknown"), (1, "uncertain", None))
        with self.assertRaises(RuntimeError):
            restarted.create("replacement", 1)

    def test_preassigned_identity_survives_process_crash_during_cancelled_create(self):
        # Mock sender: this tests durable recovery, not actual remote RTP STOP.
        # The subprocess dies while allocate() is in progress, after CANCEL.
        # Its finally/except handlers cannot write a late recovery ID.
        import subprocess
        import sys

        child_code = """
import sys, time
sys.path.insert(0, sys.argv[2])
from rt1_media_worker_lifecycle import MediaLifecycle

class PendingSender:
    @staticmethod
    def recovery_session_id(operation_id, generation):
        return f"rt1-{operation_id}-{generation}"

    def allocate(self, operation_id, generation):
        print("CREATE_ENTERED", flush=True)
        time.sleep(120)

    def stop(self, operation_id, session_id, generation):
        raise AssertionError("not reached")

MediaLifecycle(sys.argv[1], PendingSender()).create("op-crash", 1)
"""
        child = subprocess.Popen(
            [sys.executable, "-c", child_code, str(self.db), str(ROOT / "scripts")],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        try:
            self.assertEqual(child.stdout.readline().strip(), "CREATE_ENTERED")
            self.assertEqual(self.worker.status("op-crash"), (1, "creating", None))
            self.assertEqual(
                self.worker._db.execute(
                    "SELECT preassigned_session_id FROM media_operations WHERE id='op-crash'"
                ).fetchone(),
                ("rt1-op-crash-1",),
            )
            self.assertIsNone(self.worker.cancel("op-crash", 1))
            self.assertEqual(self.worker.status("op-crash"), (1, "stopping", None))
            self.assertEqual(self.sender.stopped, 0)
        finally:
            child.kill()
            child.communicate(timeout=3)

        class RecoverableSender(Sender):
            def stop(self, operation_id, session_id, generation):
                if session_id != "rt1-op-crash-1":
                    raise RuntimeError("unexpected sender identity")
                return super().stop(operation_id, session_id, generation)

        recovered_sender = RecoverableSender()
        restarted = mod.MediaLifecycle(self.db, recovered_sender)
        restarted.quarantine_after_restart()
        self.assertEqual(
            restarted.status("op-crash"), (1, "uncertain", "rt1-op-crash-1")
        )
        with self.assertRaises(RuntimeError):
            restarted.create("replacement", 1)
        restarted.reconcile()
        self.assertEqual(
            restarted.status("op-crash"), (1, "stopped", "rt1-op-crash-1")
        )
        self.assertEqual(recovered_sender.stopped, 1)

    def test_preassigned_backend_identity_mismatch_stays_quarantined(self):
        class MismatchedSender(Sender):
            @staticmethod
            def recovery_session_id(operation_id, generation):
                return "expected-sender"

            def allocate(self, operation_id, generation):
                self.created += 1
                return "unexpected-sender"

            def stop(self, operation_id, session_id, generation):
                if session_id == "expected-sender":
                    raise RuntimeError("preassigned STOP unconfirmed")
                return super().stop(operation_id, session_id, generation)

        sender = MismatchedSender()
        worker = mod.MediaLifecycle(self.db, sender)
        with self.assertRaisesRegex(RuntimeError, "identity mismatch"):
            worker.create("op-mismatch", 1)
        self.assertEqual(
            worker.status("op-mismatch"), (1, "uncertain", "unexpected-sender")
        )
        self.assertEqual(sender.stopped, 1, "unexpected sender must receive compensating STOP")
        self.assertEqual(
            worker._db.execute(
                "SELECT preassigned_session_id FROM media_operations WHERE id='op-mismatch'"
            ).fetchone(), ("expected-sender",),
        )
        restarted = mod.MediaLifecycle(self.db, sender)
        restarted.quarantine_after_restart()
        self.assertEqual(
            restarted.status("op-mismatch"), (1, "uncertain", "unexpected-sender")
        )
        with self.assertRaisesRegex(RuntimeError, "preassigned STOP unconfirmed"):
            restarted.reconcile()
        self.assertEqual(sender.stopped, 1, "actual sender STOP must not be repeated")
        self.assertEqual(restarted.status("op-mismatch")[1], "uncertain")
        with self.assertRaises(RuntimeError):
            worker.create("replacement", 1)

    def test_mismatched_create_requires_both_distinct_av_stop_receipts(self):
        # Simulated sender identities, NOT real D-ID remote STOP evidence.
        class DualSender(Sender):
            @staticmethod
            def recovery_session_id(operation_id, generation):
                return "promised-sender"

            def allocate(self, operation_id, generation):
                return "actual-sender"

            def stop(self, operation_id, session_id, generation):
                self.stop_ids.append(session_id)
                return mod.StopReceipt(operation_id, session_id, generation, True, True)

        sender = DualSender()
        sender.stop_ids = []
        worker = mod.MediaLifecycle(self.db, sender)
        with self.assertRaisesRegex(RuntimeError, "identity mismatch"):
            worker.create("dual", 1)
        self.assertEqual(sender.stop_ids, ["actual-sender", "promised-sender"])
        self.assertEqual(worker.status("dual"), (1, "stopped", "actual-sender"))
        self.assertEqual(
            worker._db.execute(
                "SELECT actual_stop_confirmed FROM media_operations WHERE id='dual'"
            ).fetchone(), (1,),
        )

    def test_mismatched_create_restarts_at_only_unconfirmed_stop_target(self):
        class RetryPromised(Sender):
            @staticmethod
            def recovery_session_id(operation_id, generation):
                return "promised-sender"

            def allocate(self, operation_id, generation):
                return "actual-sender"

            def stop(self, operation_id, session_id, generation):
                self.stop_ids.append(session_id)
                if session_id == "promised-sender" and self.fail_promised:
                    raise TimeoutError("promised sender STOP receipt lost")
                return mod.StopReceipt(operation_id, session_id, generation, True, True)

        sender = RetryPromised()
        sender.stop_ids = []
        sender.fail_promised = True
        worker = mod.MediaLifecycle(self.db, sender)
        with self.assertRaisesRegex(RuntimeError, "compensating STOP unconfirmed"):
            worker.create("dual-retry", 1)
        self.assertEqual(sender.stop_ids, ["actual-sender", "promised-sender"])
        self.assertEqual(worker.status("dual-retry")[1], "uncertain")
        self.assertEqual(
            worker._db.execute(
                "SELECT actual_stop_confirmed FROM media_operations WHERE id='dual-retry'"
            ).fetchone(), (1,),
        )
        restarted = mod.MediaLifecycle(self.db, sender)
        restarted.quarantine_after_restart()
        with self.assertRaises(RuntimeError):
            restarted.create("replacement", 1)
        sender.fail_promised = False
        restarted.reconcile()
        self.assertEqual(
            sender.stop_ids, ["actual-sender", "promised-sender", "promised-sender"],
        )
        self.assertEqual(restarted.status("dual-retry")[1], "stopped")

    def test_mismatched_partial_promised_video_stop_never_clears_quarantine(self):
        class PartialPromised(Sender):
            @staticmethod
            def recovery_session_id(operation_id, generation):
                return "promised-sender"

            def allocate(self, operation_id, generation):
                return "actual-sender"

            def stop(self, operation_id, session_id, generation):
                return mod.StopReceipt(
                    operation_id, session_id, generation, True,
                    session_id != "promised-sender",
                )

        worker = mod.MediaLifecycle(self.db, PartialPromised())
        with self.assertRaisesRegex(RuntimeError, "compensating STOP unconfirmed"):
            worker.create("dual-partial", 1)
        self.assertEqual(worker.status("dual-partial")[1], "uncertain")
        with self.assertRaisesRegex(RuntimeError, "preassigned remote audio/video STOP"):
            worker.reconcile()
        with self.assertRaises(RuntimeError):
            worker.create("replacement", 1)

    def test_mismatched_sender_stop_failure_is_durable_and_never_admits_replacement(self):
        # Simulated provider: a returned late ID is known, but remote STOP
        # fails. Neither the preassigned ID nor the unexpected ID is trusted.
        class FailedCompensation(Sender):
            @staticmethod
            def recovery_session_id(operation_id, generation):
                return "promised-sender"

            def allocate(self, operation_id, generation):
                self.created += 1
                return "late-sender"

            def stop(self, operation_id, session_id, generation):
                self.stopped += 1
                raise TimeoutError("simulated provider STOP timeout")

        sender = FailedCompensation()
        worker = mod.MediaLifecycle(self.db, sender)
        with self.assertRaisesRegex(RuntimeError, "identity mismatch"):
            worker.create("op-late", 1)
        self.assertEqual(sender.stopped, 1)
        self.assertEqual(worker.status("op-late"), (1, "uncertain", "late-sender"))
        restarted = mod.MediaLifecycle(self.db, sender)
        restarted.quarantine_after_restart()
        with self.assertRaises(TimeoutError):
            restarted.reconcile()
        self.assertEqual(sender.stopped, 2)
        self.assertEqual(restarted.status("op-late"), (1, "uncertain", "late-sender"))
        with self.assertRaises(RuntimeError):
            restarted.create("replacement", 1)

    def test_legacy_sqlite_ledger_migrates_without_discarding_unresolved_sender(self):
        import sqlite3

        legacy_path = Path(self.temp.name) / "legacy.sqlite"
        db = sqlite3.connect(legacy_path)
        try:
            db.execute("""CREATE TABLE media_operations (
                id TEXT PRIMARY KEY, generation INTEGER NOT NULL,
                phase TEXT NOT NULL, session_id TEXT,
                CHECK(phase IN ('creating', 'active', 'stopping', 'stopped', 'uncertain'))
            )""")
            db.execute(
                "INSERT INTO media_operations VALUES(?,?,?,?)",
                ("old-op", 1, "uncertain", "old-sender"),
            )
            db.commit()
        finally:
            db.close()
        migrated = mod.MediaLifecycle(legacy_path, self.sender)
        self.assertEqual(migrated.status("old-op"), (1, "uncertain", "old-sender"))
        self.assertIn(
            "preassigned_session_id",
            [row[1] for row in migrated._db.execute("PRAGMA table_info(media_operations)")],
        )
        self.assertIn(
            "actual_stop_confirmed",
            [row[1] for row in migrated._db.execute("PRAGMA table_info(media_operations)")],
        )
        with self.assertRaises(RuntimeError):
            migrated.create("replacement", 1)

    def test_late_provider_generated_id_is_stopped_before_create_error_escapes(self):
        class LateIdentified(Sender):
            def allocate(self, operation_id, generation):
                self.created += 1
                raise mod.AllocatedSessionError(operation_id, generation, "late-did-id")

        sender = LateIdentified()
        worker = mod.MediaLifecycle(self.db, sender)
        with self.assertRaises(mod.AllocatedSessionError):
            worker.create("op-late", 1)
        self.assertEqual(sender.stopped, 1)
        self.assertEqual(worker.status("op-late"), (1, "stopped", "late-did-id"))
        with self.assertRaises(RuntimeError):
            worker.create("op-late", 1)

    def test_late_provider_generated_id_failed_stop_retries_after_restart(self):
        class LateIdentified(Sender):
            def __init__(self):
                super().__init__()
                self.fail_stop = True

            def allocate(self, operation_id, generation):
                self.created += 1
                raise mod.AllocatedSessionError(operation_id, generation, "late-did-id")

            def stop(self, operation_id, session_id, generation):
                if self.fail_stop:
                    self.stopped += 1
                    raise TimeoutError("STOP reply lost")
                return super().stop(operation_id, session_id, generation)

        sender = LateIdentified()
        worker = mod.MediaLifecycle(self.db, sender)
        with self.assertRaises(mod.AllocatedSessionError):
            worker.create("op-late", 1)
        self.assertEqual(worker.status("op-late"), (1, "uncertain", "late-did-id"))
        with self.assertRaises(RuntimeError):
            worker.create("replacement", 1)
        sender.fail_stop = False
        restarted = mod.MediaLifecycle(self.db, sender)
        restarted.quarantine_after_restart()
        restarted.reconcile()
        self.assertEqual(restarted.status("op-late"), (1, "stopped", "late-did-id"))
        self.assertEqual(sender.stopped, 2)

    def test_late_id_mismatch_requires_stop_of_both_possible_senders(self):
        class TwoIdentities(Sender):
            def __init__(self):
                super().__init__()
                self.targets = []

            def recovery_session_id(self, operation_id, generation):
                return "promised-id"

            def allocate(self, operation_id, generation):
                self.created += 1
                raise mod.AllocatedSessionError(operation_id, generation, "actual-id")

            def stop(self, operation_id, session_id, generation):
                self.targets.append(session_id)
                return mod.StopReceipt(operation_id, session_id, generation, True, True)

        sender = TwoIdentities()
        worker = mod.MediaLifecycle(self.db, sender)
        with self.assertRaises(mod.AllocatedSessionError):
            worker.create("op-late", 1)
        self.assertEqual(sender.targets, ["actual-id", "promised-id"])
        self.assertEqual(worker.status("op-late"), (1, "stopped", "actual-id"))

    def test_foreign_late_provider_identity_cannot_authorize_remote_stop(self):
        class ForeignIdentity(Sender):
            def allocate(self, operation_id, generation):
                self.created += 1
                raise mod.AllocatedSessionError("other-operation", generation, "foreign-id")

        sender = ForeignIdentity()
        worker = mod.MediaLifecycle(self.db, sender)
        with self.assertRaises(mod.AllocatedSessionError):
            worker.create("op-late", 1)
        self.assertEqual(worker.status("op-late"), (1, "uncertain", None))
        self.assertEqual(sender.stopped, 0)
        restarted = mod.MediaLifecycle(self.db, sender)
        restarted.quarantine_after_restart()
        restarted.reconcile()
        self.assertEqual(restarted.status("op-late"), (1, "uncertain", None))
        with self.assertRaises(RuntimeError):
            restarted.create("replacement", 1)

    def test_unknown_id_after_restart_must_not_reallocate(self):
        self.worker._write("op-a", 1, "uncertain", None)
        restarted = mod.MediaLifecycle(self.db, self.sender)
        restarted.reconcile()
        with self.assertRaises(RuntimeError):
            restarted.create("op-b", 1)
        self.assertEqual(self.sender.created, 0)


if __name__ == "__main__":
    unittest.main()
