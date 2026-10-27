"""Contract tests for operation-owned avatar allocation and sender STOP."""
import importlib.util
from pathlib import Path
import threading
import unittest

spec = importlib.util.spec_from_file_location(
    "worker_lifecycle", Path(__file__).with_name("worker_lifecycle.py")
)
module = importlib.util.module_from_spec(spec)
import sys
sys.modules[spec.name] = module
spec.loader.exec_module(module)
MediaLifecycle = module.MediaLifecycle
MediaAllocation = module.MediaAllocation
LifecycleError = module.LifecycleError


class Sender:
    def __init__(self, stopped=True):
        self.stopped = stopped
        self.requested = False

    def stop(self):
        self.requested = True

    def wait_stopped(self, timeout):
        return self.requested and self.stopped


class LifecycleTests(unittest.TestCase):
    def test_duplicate_create_never_allocates_twice(self):
        lifecycle = MediaLifecycle()
        count = []
        def allocate(cancel):
            count.append(1)
            return MediaAllocation("session-1", Sender(), Sender())
        lifecycle.create("operation-1", allocate)
        with self.assertRaises(LifecycleError):
            lifecycle.create("operation-1", allocate)
        self.assertEqual(len(count), 1)
        self.assertTrue(lifecycle.stop("session-1", 0.1)["video_stopped"])

    def test_cancel_before_create_keeps_tombstone(self):
        lifecycle = MediaLifecycle()
        receipt = lifecycle.cancel("operation-1", 0.1)
        self.assertEqual(receipt, {
            "operation_id": "operation-1",
            "cancelled_before_create": True,
        })
        # No sender existed: never fabricate an audio/video STOP receipt.
        self.assertNotIn("audio_stopped", receipt)
        self.assertNotIn("video_stopped", receipt)
        again = lifecycle.cancel("operation-1", 0.1)
        self.assertEqual(again, {"cancelled_before_create": True})
        with self.assertRaises(LifecycleError):
            lifecycle.create("operation-1", lambda _: self.fail("unexpected allocation"))

    def test_cancel_during_allocation_stops_both_senders(self):
        lifecycle = MediaLifecycle()
        entered = threading.Event()
        release = threading.Event()
        audio, video = Sender(), Sender()
        errors = []
        def allocate(cancel):
            entered.set()
            release.wait(2)
            return MediaAllocation("session-1", audio, video)
        worker = threading.Thread(target=lambda: self._attempt(
            lambda: lifecycle.create("operation-1", allocate), errors
        ))
        worker.start()
        self.assertTrue(entered.wait(1))
        with self.assertRaisesRegex(LifecycleError, "unconfirmed"):
            lifecycle.cancel("operation-1", 0.01)
        release.set()
        worker.join(2)
        self.assertFalse(worker.is_alive())
        self.assertEqual(len(errors), 1)
        self.assertTrue(audio.requested)
        self.assertTrue(video.requested)
        receipt = lifecycle.cancel("operation-1", 0.1)
        self.assertTrue(receipt["audio_stopped"])
        self.assertTrue(receipt["video_stopped"])

    def test_stop_denies_one_active_sender(self):
        lifecycle = MediaLifecycle()
        audio, video = Sender(), Sender(stopped=False)
        lifecycle.create("operation-1", lambda _: MediaAllocation("session-1", audio, video))
        with self.assertRaisesRegex(LifecycleError, "unconfirmed"):
            lifecycle.stop("session-1", 0.1)
        self.assertTrue(audio.requested)
        self.assertTrue(video.requested)
        video.stopped = True
        self.assertTrue(lifecycle.stop("session-1", 0.1)["video_stopped"])

    def test_audio_stop_failure_does_not_skip_video_stop(self):
        lifecycle = MediaLifecycle()
        class FailedAudio(Sender):
            def stop(self):
                raise RuntimeError("audio failed")
        audio, video = FailedAudio(), Sender()
        lifecycle.create("operation-1", lambda _: MediaAllocation("session-1", audio, video))
        with self.assertRaisesRegex(LifecycleError, "unconfirmed"):
            lifecycle.stop("session-1", 0.1)
        self.assertTrue(video.requested)

    @staticmethod
    def _attempt(fn, errors):
        try:
            fn()
        except BaseException as error:
            errors.append(error)


if __name__ == "__main__":
    unittest.main()
