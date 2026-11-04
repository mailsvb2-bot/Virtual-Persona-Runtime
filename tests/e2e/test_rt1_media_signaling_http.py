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
        # Real HTTP requests race. A provider can activate RTP at the END of
        # answer, so STOP must not be acknowledged while SDP is still in flight.
        answer_entered = threading.Event()
        release_answer = threading.Event()

        def slow_answer(operation_id, session_id, generation, sdp):
            answer_entered.set()
            if not release_answer.wait(3):
                raise RuntimeError("test answer did not release")
            # Deliberately (re)activate the sender as a late provider side
            # effect. The old status-only fence allowed STOP to finish first.
            self.sender.active = True
            self.sender.answers += 1

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
            self.assertTrue(answer_entered.wait(3), "answer must reach backend")
            status, receipt = self.req("DELETE", body={"generation": 1})
            self.assertEqual(status, 409, "STOP must not race ahead of active SDP")
            self.assertFalse(receipt["stop_confirmed"])
            self.assertEqual(self.coordinator.status("op-1")[1], "stopping")
        finally:
            release_answer.set()
            caller.join(timeout=4)
        self.assertFalse(caller.is_alive(), "answer must not hang after owner revoke")
        self.assertEqual(len(answers), 1)
        self.assertEqual(answers[0][0], 409, "revoked answer must not receive success")
        stopped, receipt = self.req("DELETE", body={"generation": 1})
        self.assertEqual(stopped, 200, "retry must reconcile the durable STOP")
        self.assertTrue(receipt["audio_stopped"])
        self.assertTrue(receipt["video_stopped"])
        self.assertFalse(self.sender.active, "late SDP must not resurrect RTP after STOP")
        self.assertEqual(self.sender.answers, 1)
        self.assertEqual(self.req("GET", "/offer")[0], 410)

    def test_late_backend_success_cannot_ack_revoked_answer(self):
        # Even a provider reporting success after owner revoke cannot expose
        # the answer as accepted or overtake the canonical STOP receipt.
        answer_entered = threading.Event()
        release_answer = threading.Event()
        backend_returned_success = threading.Event()

        def late_successful_answer(operation_id, session_id, generation, sdp):
            answer_entered.set()
            if not release_answer.wait(3):
                raise RuntimeError("test answer did not release")
            backend_returned_success.set()
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
            self.assertEqual(stopped, 409, "STOP cannot confirm before SDP finishes")
            self.assertFalse(receipt["stop_confirmed"])
        finally:
            release_answer.set()
            caller.join(timeout=4)

        self.assertFalse(caller.is_alive(), "answer must finish after revoke")
        self.assertTrue(backend_returned_success.is_set(), "exercise late successful return")
        self.assertEqual(len(answers), 1)
        self.assertEqual(answers[0][0], 409, "stale provider success must not be acknowledged")
        stopped, receipt = self.req("DELETE", body={"generation": 1})
        self.assertEqual(stopped, 200)
        self.assertTrue(receipt["audio_stopped"])
        self.assertTrue(receipt["video_stopped"])
        self.assertFalse(self.sender.active)
        self.assertEqual(self.sender.answers, 0)
        self.assertEqual(self.req("GET", "/offer")[0], 410)

    def test_in_flight_offer_is_not_exposed_after_confirmed_stop(self):
        offer_entered = threading.Event()
        release_offer = threading.Event()
        original_offer = self.sender.offer

        def delayed_offer(operation_id, session_id, generation):
            # Produce a valid offer before revoke, then hold it until STOP
            # confirms. The HTTP layer must re-check durable authorization.
            result = original_offer(operation_id, session_id, generation)
            offer_entered.set()
            if not release_offer.wait(3):
                raise RuntimeError("test offer did not release")
            return result

        self.sender.offer = delayed_offer
        self.assertEqual(self.req("PUT", body={"generation": 1})[0], 200)
        offers = []

        def fetch_offer():
            offers.append(self.req("GET", "/offer"))

        caller = threading.Thread(target=fetch_offer)
        caller.start()
        try:
            self.assertTrue(offer_entered.wait(3), "offer must reach backend")
            stopped, receipt = self.req("DELETE", body={"generation": 1})
            self.assertEqual(stopped, 200)
            self.assertTrue(receipt["audio_stopped"])
            self.assertTrue(receipt["video_stopped"])
        finally:
            release_offer.set()
            caller.join(timeout=4)

        self.assertFalse(caller.is_alive(), "offer must finish after STOP")
        self.assertEqual(len(offers), 1)
        self.assertEqual(offers[0][0], 410, "stale offer cannot be published")
        self.assertNotIn("sdp", offers[0][1])
        self.assertFalse(self.sender.active)
        self.assertEqual(self.req("GET", "/offer")[0], 410)

    def test_malformed_answer_never_dispatches_or_revokes_live_sender(self):
        # Bad JSON shape is a client validation error, not provider failure.
        self.assertEqual(self.req("PUT", body={"generation": 1})[0], 200)
        for invalid in (["not-an-object"], {"type": "answer", "sdp": ""}):
            status, _ = self.req("POST", "/answer", invalid)
            self.assertEqual(status, 400)
        self.assertEqual(self.coordinator.status("op-1")[1], "active")
        self.assertTrue(self.sender.active)
        self.assertEqual(self.sender.answers, 0)
        self.assertEqual(self.req("DELETE", body={"generation": 1})[0], 200)

    def test_provider_value_error_after_answer_revokes_both_senders(self):
        # Provider-side ValueError is NOT an invalid-request exemption: SDP
        # may already have taken effect before the adapter reports failure.
        self.assertEqual(self.req("PUT", body={"generation": 1})[0], 200)

        def failed_after_accept(operation_id, session_id, generation, sdp):
            self.sender.answers += 1
            raise ValueError("simulated post-SDP provider failure")

        self.sender.answer = failed_after_accept
        status, body = self.req("POST", "/answer", {"type": "answer", "sdp": "v=0"})
        self.assertEqual(status, 409)
        self.assertNotIn("simulated", str(body))
        self.assertEqual(self.sender.answers, 1)
        self.assertFalse(self.sender.active, "both sender STOPs must run")
        self.assertEqual(self.coordinator.status("op-1")[1], "stopped")
        self.assertEqual(self.req("GET", "/offer")[0], 410)
        self.assertEqual(self.req("PUT", body={"generation": 1})[0], 409)

    def test_answer_timeout_partial_video_stop_quarantines_across_restart(self):
        # Simulated provider timeout and incomplete VIDEO stop, not D-ID proof.
        self.assertEqual(self.req("PUT", body={"generation": 1})[0], 200)
        original_stop = self.sender.stop

        def failed_answer(operation_id, session_id, generation, sdp):
            raise TimeoutError("simulated provider stalled after SDP")

        def partial_stop(operation_id, session_id, generation):
            return StopReceipt(operation_id, session_id, generation, True, False)

        self.sender.answer = failed_answer
        self.sender.stop = partial_stop
        status, _ = self.req("POST", "/answer", {"type": "answer", "sdp": "v=0"})
        self.assertEqual(status, 409)
        self.assertEqual(self.coordinator.status("op-1")[1], "uncertain")
        self.assertEqual(self.req("GET", "/offer")[0], 410)
        self.assertEqual(self.req("PUT", body={"generation": 1})[0], 409)

        # New process/connection sees the same durable unresolved identity.
        recovered = MediaLifecycle(Path(self.tmp.name) / "ledger.db", self.sender)
        recovered.quarantine_after_restart()
        self.assertEqual(recovered.status("op-1")[1], "uncertain")
        self.sender.stop = original_stop
        recovered.reconcile()
        self.assertEqual(recovered.status("op-1")[1], "stopped")
        self.assertFalse(self.sender.active)

    def test_no_signaling_without_token(self):
        self.assertEqual(self.req("PUT", body={"generation": 1}, token="invalid")[0], 401)
        self.assertEqual(self.req("GET", "/offer", token="invalid")[0], 401)
        self.assertEqual(self.req("POST", "/answer", {"type": "answer", "sdp": "x"}, token="invalid")[0], 401)
        self.assertFalse(self.sender.active)


if __name__ == "__main__":
    unittest.main()
