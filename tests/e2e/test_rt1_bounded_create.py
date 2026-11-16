"""Transport-bounded CREATE and late STOP tests; simulated sender, not D-ID RTP."""
import json
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from http.server import ThreadingHTTPServer
from urllib.error import HTTPError
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from rt1_media_worker_lifecycle import MediaLifecycle, StopReceipt
from rt1_media_worker_http import handler_for


class SlowBackend:
    def __init__(self, fail=False):
        self.entered = threading.Event()
        self.release = threading.Event()
        self.cancel_seen = threading.Event()
        self.fail = fail
        self.allocations = 0
        self.stops = 0

    def persona_video_ready(self):
        return True

    def allocate(self, operation_id, generation):
        raise AssertionError("legacy unbounded allocate must not be called")

    def allocate_bounded(self, operation_id, generation, timeout, cancel_requested):
        self.allocations += 1
        self.entered.set()
        deadline = time.monotonic() + timeout
        while True:
            if cancel_requested.is_set():
                self.cancel_seen.set()
                # Simulate a provider allocating just as cancellation wins.
                return "real-late-id"
            if self.fail:
                raise TimeoutError("simulated provider outcome unknown")
            if self.release.is_set():
                return "real-late-id"
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("simulated provider transport deadline")
            time.sleep(min(0.005, remaining))

    def stop(self, operation_id, session_id, generation):
        self.stops += 1
        return StopReceipt(operation_id, session_id, generation, True, True)


class BoundedCreateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.backend = SlowBackend()
        self.lifecycle = MediaLifecycle(Path(self.temp.name) / "ledger.sqlite", self.backend)

    def tearDown(self):
        self.backend.release.set()
        self.temp.cleanup()

    def test_provider_transport_deadline_quarantines_unknown_create_without_waiter_thread(self):
        started = time.monotonic()
        with self.assertRaisesRegex(RuntimeError, "CREATE deadline exceeded"):
            self.lifecycle.create_bounded("bounded", 1, 0.2)
        self.assertLess(time.monotonic() - started, 1)
        self.assertTrue(self.backend.entered.is_set())
        self.assertEqual(self.lifecycle.status("bounded"), (1, "uncertain", None))
        self.assertEqual(self.backend.allocations, 1)
        self.assertEqual(self.backend.stops, 0)
        self.assertFalse(
            any(thread.name == "rt1-bounded-create" for thread in threading.enumerate())
        )
        with self.assertRaisesRegex(RuntimeError, "unresolved media allocation"):
            self.lifecycle.create("replacement", 1)

    def test_owner_cancel_reaches_provider_and_late_session_id_is_compensated(self):
        outcome = []

        def create():
            try:
                self.lifecycle.create_bounded("bounded", 1, 2.0)
            except Exception as error:
                outcome.append(error)

        task = threading.Thread(target=create, name="test-create-caller")
        task.start()
        self.assertTrue(self.backend.entered.wait(1))
        self.assertIsNone(self.lifecycle.cancel("bounded", 1))
        task.join(2)
        self.assertFalse(task.is_alive())
        self.assertTrue(self.backend.cancel_seen.is_set())
        self.assertEqual(len(outcome), 1)
        self.assertRegex(str(outcome[0]), "operation revoked during allocation")
        self.assertEqual(self.lifecycle.status("bounded"), (1, "stopped", "real-late-id"))
        self.assertEqual(self.backend.stops, 1)

    def test_lost_unknown_id_remains_quarantined_after_provider_timeout(self):
        self.backend.fail = True
        with self.assertRaisesRegex(TimeoutError, "provider outcome unknown"):
            self.lifecycle.create_bounded("bounded", 1, 0.2)
        self.assertEqual(self.lifecycle.status("bounded"), (1, "uncertain", None))
        self.assertEqual(self.backend.stops, 0)
        with self.assertRaises(RuntimeError):
            self.lifecycle.create("replacement", 1)
        restarted = MediaLifecycle(Path(self.temp.name) / "ledger.sqlite", self.backend)
        restarted.quarantine_after_restart()
        self.assertEqual(restarted.status("bounded"), (1, "uncertain", None))

    def test_deferred_cancel_of_already_active_session_requires_reconcile(self):
        self.backend.release.set()
        self.assertEqual(self.lifecycle.create("bounded", 1), "real-late-id")
        self.assertIsNone(self.lifecycle.cancel("bounded", 1, wait_for_stop=False))
        self.assertEqual(self.lifecycle.status("bounded")[1], "stopping")
        self.assertEqual(self.backend.stops, 0)
        self.lifecycle.reconcile()
        self.assertEqual(self.lifecycle.status("bounded")[1], "stopped")
        self.assertEqual(self.backend.stops, 1)

    def test_hung_stop_does_not_hold_bounded_delete_or_fabricate_receipt(self):
        self.backend.release.set()
        self.lifecycle.create("bounded", 1)
        stop_entered = threading.Event()
        stop_release = threading.Event()
        original_stop = self.backend.stop

        def blocked_stop(*args):
            stop_entered.set()
            stop_release.wait(2)
            return original_stop(*args)

        try:
            self.backend.stop = blocked_stop
            started = time.monotonic()
            self.assertIsNone(self.lifecycle.cancel_bounded("bounded", 1, 0.02))
            self.assertLess(time.monotonic() - started, 0.15)
            self.assertTrue(stop_entered.wait(1))
            self.assertEqual(self.lifecycle.status("bounded")[1], "stopping")
        finally:
            stop_release.set()
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline and self.lifecycle.status("bounded")[1] != "stopped":
            time.sleep(0.01)
        self.assertEqual(self.lifecycle.status("bounded")[1], "stopped")

    def test_invalid_deadline_never_dispatches_provider(self):
        for timeout in (0, -1, 61):
            with self.assertRaises(ValueError):
                self.lifecycle.create_bounded("bounded", 1, timeout)
        self.assertEqual(self.backend.allocations, 0)
        self.assertIsNone(self.lifecycle.status("bounded"))


class BoundedHttpCreateTests(unittest.TestCase):
    def test_authenticated_put_returns_transport_bounded_failure(self):
        with tempfile.TemporaryDirectory() as temp:
            backend = SlowBackend()
            lifecycle = MediaLifecycle(Path(temp) / "http.sqlite", backend)
            server = ThreadingHTTPServer(
                ("127.0.0.1", 0),
                handler_for(lifecycle, "secret", create_timeout_seconds=0.2),
            )
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            url = f"http://127.0.0.1:{server.server_port}/v1/avatar/operations/slow"
            request = Request(
                url,
                method="PUT",
                data=json.dumps({"generation": 1}).encode(),
                headers={"Authorization": "Bearer secret", "Content-Type": "application/json"},
            )
            try:
                started = time.monotonic()
                with self.assertRaises(HTTPError) as response:
                    urlopen(request, timeout=2)
                self.assertEqual(response.exception.code, 409)
                self.assertLess(time.monotonic() - started, 1)
                self.assertEqual(lifecycle.status("slow"), (1, "uncertain", None))
                self.assertEqual(backend.allocations, 1)
                self.assertFalse(
                    any(item.name == "rt1-bounded-create" for item in threading.enumerate())
                )
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=2)


if __name__ == "__main__":
    unittest.main()
