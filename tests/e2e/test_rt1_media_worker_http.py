"""HTTP boundary integration against the durable operation ledger (mock backend).

A separate deployed test must inspect real sender RTP on revoke.
"""
import json
import sys
import tempfile
import threading
import unittest
from http.server import ThreadingHTTPServer
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from rt1_media_worker_http import handler_for
from rt1_media_worker_lifecycle import MediaLifecycle, StopReceipt


class Backend:
    def __init__(self):
        self.allocations = 0
        self.stops = 0

    def allocate(self, operation_id, generation):
        self.allocations += 1
        return "media-1"

    def stop(self, operation_id, session_id, generation):
        self.stops += 1
        return StopReceipt(operation_id, session_id, generation, True, True)


class HttpLifecycleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.backend = Backend()
        self.lifecycle = MediaLifecycle(Path(self.temp.name) / "ledger.sqlite", self.backend)
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), handler_for(self.lifecycle, "test-secret"))
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.url = f"http://127.0.0.1:{self.server.server_port}/v1/avatar/operations/operation-1"

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)
        self.temp.cleanup()

    def request(self, method, token="test-secret", generation=1):
        request = Request(self.url, method=method,
                          data=json.dumps({"generation": generation}).encode(),
                          headers={"Authorization": "Bearer " + token,
                                   "Content-Type": "application/json"})
        try:
            with urlopen(request, timeout=3) as response:
                return response.status, json.load(response)
        except HTTPError as error:
            return error.code, json.load(error)

    def test_authentication_idempotent_create_and_confirmed_remote_stop(self):
        self.assertEqual(self.request("PUT", token="wrong")[0], 401)
        self.assertEqual(self.backend.allocations, 0)
        self.assertEqual(self.request("PUT")[0], 200)
        self.assertEqual(self.request("PUT")[0], 200)
        self.assertEqual(self.backend.allocations, 1)
        status, stopped = self.request("DELETE")
        self.assertEqual(status, 200)
        self.assertIs(stopped["audio_stopped"], True)
        self.assertIs(stopped["video_stopped"], True)
        self.assertEqual(self.backend.stops, 1)
        self.assertEqual(self.request("PUT")[0], 409)

    def test_precreate_cancel_is_tombstone_not_fake_stop_receipt(self):
        status, body = self.request("DELETE")
        self.assertEqual(status, 202)
        self.assertIs(body["stop_confirmed"], False)
        self.assertEqual(self.request("PUT")[0], 409)
        self.assertEqual(self.backend.allocations, 0)


if __name__ == "__main__":
    unittest.main()
