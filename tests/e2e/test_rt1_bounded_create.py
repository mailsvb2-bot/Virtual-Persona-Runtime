"""Bounded CREATE and late STOP tests using a simulated backend, not real D-ID RTP."""
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
        self.fail = fail
        self.allocations = 0
        self.stops = 0

    def allocate(self, operation_id, generation):
        self.allocations += 1
        self.entered.set()
        if not self.release.wait(3):
            raise RuntimeError("simulated hung CREATE")
        if self.fail:
            raise RuntimeError("simulated lost provider ID")
        return "real-late-id"

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

    def await_phase(self, phase):
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            if self.lifecycle.status("bounded")[1] == phase:
                return
            time.sleep(0.01)
        self.fail(f"late CREATE did not reach durable {phase}")

    def test_deadline_fences_late_create_and_compensates_both_senders(self):
        started = time.monotonic()
        try:
            with self.assertRaisesRegex(RuntimeError, "CREATE deadline exceeded"):
                self.lifecycle.create_bounded("bounded", 1, 0.05)
            self.assertLess(time.monotonic() - started, 1)
            self.assertTrue(self.backend.entered.is_set())
            self.assertEqual(self.lifecycle.status("bounded"), (1, "stopping", None))
            with self.assertRaises(RuntimeError):
                self.lifecycle.create("replacement", 1)
        finally:
            self.backend.release.set()
        self.await_phase("stopped")
        self.assertEqual(self.lifecycle.status("bounded"), (1, "stopped", "real-late-id"))
        self.assertEqual(self.backend.allocations, 1)
        self.assertEqual(self.backend.stops, 1)
        receipt = self.lifecycle.cancel("bounded", 1)
        self.assertTrue(receipt.audio_stopped and receipt.video_stopped)

    def test_lost_unknown_id_remains_quarantined_after_deadline(self):
        self.backend.fail = True
        try:
            with self.assertRaisesRegex(RuntimeError, "CREATE deadline exceeded"):
                self.lifecycle.create_bounded("bounded", 1, 0.05)
        finally:
            self.backend.release.set()
        self.await_phase("uncertain")
        self.assertEqual(self.lifecycle.status("bounded"), (1, "uncertain", None))
        self.assertEqual(self.backend.stops, 0)
        with self.assertRaises(RuntimeError):
            self.lifecycle.create("replacement", 1)
        restarted = MediaLifecycle(self.lifecycle._db.execute("PRAGMA database_list").fetchone()[2], self.backend)
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

    def test_invalid_deadline_never_dispatches_provider(self):
        for timeout in (0, -1, 61):
            with self.assertRaises(ValueError):
                self.lifecycle.create_bounded("bounded", 1, timeout)
        self.assertEqual(self.backend.allocations, 0)
        self.assertIsNone(self.lifecycle.status("bounded"))



class BoundedHttpCreateTests(unittest.TestCase):
    def test_authenticated_put_returns_bounded_failure_and_late_stop(self):
        with tempfile.TemporaryDirectory() as temp:
            backend = SlowBackend()
            lifecycle = MediaLifecycle(Path(temp) / "http.sqlite", backend)
            server = ThreadingHTTPServer(
                ("127.0.0.1", 0),
                handler_for(lifecycle, "secret", create_timeout_seconds=0.05),
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
                self.assertEqual(lifecycle.status("slow"), (1, "stopping", None))
                backend.release.set()
                deadline = time.monotonic() + 2
                while time.monotonic() < deadline and lifecycle.status("slow")[1] != "stopped":
                    time.sleep(0.01)
                self.assertEqual(lifecycle.status("slow"), (1, "stopped", "real-late-id"))
                self.assertEqual(backend.stops, 1)
            finally:
                backend.release.set()
                server.shutdown()
                server.server_close()
                thread.join(timeout=2)


if __name__ == "__main__":
    unittest.main()
