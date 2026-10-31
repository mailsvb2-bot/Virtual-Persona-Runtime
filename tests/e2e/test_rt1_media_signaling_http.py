"""Canonical worker signaling is denied after actual STOP and before CREATE."""
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


class Sender:
    def __init__(self):
        self.active = False
        self.answers = 0

    def allocate(self, operation_id, generation):
        self.active = True
        return "session-1"

    def offer(self, operation_id, session_id, generation):
        if not self.active:
            raise RuntimeError("sender closed")
        return {"type": "offer", "sdp": "test-offer"}

    def answer(self, operation_id, session_id, generation, sdp):
        if not self.active:
            raise RuntimeError("sender closed")
        self.answers += 1

    def stop(self, operation_id, session_id, generation):
        self.active = False
        return StopReceipt(operation_id, session_id, generation, True, True)


class SignalingTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.sender = Sender()
        self.coordinator = MediaLifecycle(Path(self.tmp.name) / "ledger.db", self.sender)
        self.server = ThreadingHTTPServer(
            ("127.0.0.1", 0), handler_for(self.coordinator, "secret-token")
        )
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.root = f"http://127.0.0.1:{self.server.server_port}/v1/avatar/operations/op-1"

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)
        self.tmp.cleanup()

    def req(self, method, suffix="", body=None, token="secret-token"):
        payload = json.dumps(body).encode() if body is not None else None
        request = Request(
            self.root + suffix, method=method, data=payload,
            headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"},
        )
        try:
            with urlopen(request, timeout=3) as response:
                return response.status, json.load(response)
        except HTTPError as error:
            return error.code, json.load(error)

    def test_offer_answer_and_revocation_no_resurrection(self):
        self.assertEqual(self.req("GET", "/offer")[0], 410)
        self.assertEqual(self.req("PUT", body={"generation": 1})[0], 200)
        status, offer = self.req("GET", "/offer")
        self.assertEqual(status, 200)
        self.assertEqual(offer["sdp"], "test-offer")
        self.assertEqual(self.req("POST", "/answer", {"type": "answer", "sdp": "ok"})[0], 200)
        self.assertEqual(self.sender.answers, 1)
        self.assertEqual(self.req("DELETE", body={"generation": 1})[0], 200)
        self.assertFalse(self.sender.active)
        self.assertEqual(self.req("GET", "/offer")[0], 410)
        self.assertEqual(self.req("POST", "/answer", {"type": "answer", "sdp": "late"})[0], 410)
        self.assertEqual(self.req("PUT", body={"generation": 1})[0], 409)
        self.assertEqual(self.sender.answers, 1)

    def test_concurrent_answer_and_delete_never_acknowledges_revoked_session(self):
        # Two actual HTTP requests race on ThreadingHTTPServer. An in-flight
        # backend answer must not be acknowledged after STOP wins, and the
        # already allocated sender must be terminated with both receipts.
        answer_entered = threading.Event()
        release_answer = threading.Event()
        original_answer = self.sender.answer

        def slow_answer(operation_id, session_id, generation, sdp):
            answer_entered.set()
            if not release_answer.wait(3):
                raise RuntimeError("test answer did not release")
            # This test sender does not re-open a stopped media session.
            return original_answer(operation_id, session_id, generation, sdp)

        self.sender.answer = slow_answer
        self.assertEqual(self.req("PUT", body={"generation": 1})[0], 200)
        answers = []

        def post_answer():
            answers.append(self.req(
                "POST", "/answer", {"type": "answer", "sdp": "v=0 raced"}
            ))

        caller = threading.Thread(target=post_answer)
        caller.start()
        try:
            self.assertTrue(answer_entered.wait(3), "the answer must reach backend")
            stopped, receipt = self.req("DELETE", body={"generation": 1})
            self.assertEqual(stopped, 200)
            self.assertTrue(receipt["audio_stopped"])
            self.assertTrue(receipt["video_stopped"])
        finally:
            release_answer.set()
            caller.join(timeout=4)
        self.assertFalse(caller.is_alive(), "answer must not hang after owner STOP")
        self.assertEqual(len(answers), 1)
        self.assertEqual(answers[0][0], 409, "revoked answer must not receive success")
        self.assertFalse(self.sender.active)
        self.assertEqual(self.sender.answers, 0)
        self.assertEqual(self.req("GET", "/offer")[0], 410)

    def test_late_backend_success_cannot_ack_revoked_answer(self):
        # A worker can finish an already in-flight Answer with a successful
        # return even after STOP. The HTTP layer must not turn that stale
        # provider result into an accepted visitor signaling response.
        answer_entered = threading.Event()
        release_answer = threading.Event()
        backend_returned_success = threading.Event()

        def late_successful_answer(operation_id, session_id, generation, sdp):
            answer_entered.set()
            if not release_answer.wait(3):
                raise RuntimeError("test answer did not release")
            backend_returned_success.set()
            # Simulate an already-in-flight successful provider result. Do not
            # mutate sender state: STOP has already closed the local sender.
            return None

        self.sender.answer = late_successful_answer
        self.assertEqual(self.req("PUT", body={"generation": 1})[0], 200)
        answers = []

        def post_answer():
            answers.append(self.req(
                "POST", "/answer", {"type": "answer", "sdp": "v=0 late-success"}
            ))

        caller = threading.Thread(target=post_answer)
        caller.start()
        try:
            self.assertTrue(answer_entered.wait(3), "answer must reach backend")
            stopped, receipt = self.req("DELETE", body={"generation": 1})
            self.assertEqual(stopped, 200)
            self.assertTrue(receipt["audio_stopped"])
            self.assertTrue(receipt["video_stopped"])
        finally:
            release_answer.set()
            caller.join(timeout=4)

        self.assertFalse(caller.is_alive(), "answer must finish after STOP")
        self.assertTrue(backend_returned_success.is_set(), "exercise late successful return")
        self.assertEqual(len(answers), 1)
        self.assertEqual(answers[0][0], 410, "stale provider success must not be acknowledged")
        self.assertFalse(self.sender.active)
        self.assertEqual(self.sender.answers, 0)
        self.assertEqual(self.req("GET", "/offer")[0], 410)

    def test_no_signaling_without_token(self):
        self.assertEqual(self.req("PUT", body={"generation": 1}, token="invalid")[0], 401)
        self.assertEqual(self.req("GET", "/offer", token="invalid")[0], 401)
        self.assertEqual(self.req("POST", "/answer", {"type": "answer", "sdp": "x"}, token="invalid")[0], 401)
        self.assertFalse(self.sender.active)


if __name__ == "__main__":
    unittest.main()
