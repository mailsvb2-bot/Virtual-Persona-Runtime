"""HTTP boundary integration against the durable operation ledger (mock backend).

A separate deployed test must inspect real sender RTP on revoke.
"""
import json
import sys
import tempfile
import threading
import time
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
        self.server = ThreadingHTTPServer(
            ("127.0.0.1", 0),
            handler_for(self.lifecycle, "test-secret", stop_timeout_seconds=1.0),
        )
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

    def test_authenticated_durable_status_never_invents_stop_receipt(self):
        request = Request(self.url, headers={"Authorization": "Bearer test-secret"})
        try:
            urlopen(request, timeout=3)
            self.fail("unknown operation unexpectedly resolved")
        except HTTPError as error:
            self.assertEqual(error.code, 404)
        status, created = self.request("PUT")
        self.assertEqual(status, 200)
        with urlopen(request, timeout=3) as response:
            current = json.load(response)
        self.assertEqual(current["phase"], "active")
        self.assertIs(current["stop_confirmed"], False)
        self.assertEqual(current["session_id"], created["session_id"])
        self.assertEqual(self.request("DELETE")[0], 200)
        with urlopen(request, timeout=3) as response:
            stopped = json.load(response)
        self.assertEqual(stopped["phase"], "stopped")
        self.assertIs(stopped["stop_confirmed"], True)

    def test_failed_remote_video_stop_quarantines_operation_and_denies_recreate(self):
        original_stop = self.backend.stop

        def partial_stop(operation_id, session_id, generation):
            self.backend.stops += 1
            return StopReceipt(operation_id, session_id, generation, True, False)

        self.backend.stop = partial_stop
        status, created = self.request("PUT")
        self.assertEqual(status, 200)
        self.assertEqual(created["session_id"], "media-1")
        status, error = self.request("DELETE")
        self.assertEqual(status, 409)
        self.assertIs(error["stop_confirmed"], False)

        status_request = Request(
            self.url, headers={"Authorization": "Bearer test-secret"}
        )
        with urlopen(status_request, timeout=3) as response:
            state = json.load(response)
        self.assertEqual(state["phase"], "uncertain")
        self.assertIs(state["stop_confirmed"], False)
        self.assertEqual(self.request("PUT")[0], 409)
        self.assertEqual(self.backend.allocations, 1)

        self.backend.stop = original_stop
        status, confirmed = self.request("DELETE")
        self.assertEqual(status, 200)
        self.assertIs(confirmed["audio_stopped"], True)
        self.assertIs(confirmed["video_stopped"], True)

    def test_post_to_operation_resource_cannot_cancel_or_tombstone(self):
        # POST is only valid at /answer. A POST to the operation resource
        # must not silently become DELETE, before or after allocation.
        self.assertEqual(self.request("POST")[0], 405)
        self.assertEqual(self.request("PUT")[0], 200)
        self.assertEqual(self.backend.allocations, 1)
        self.assertEqual(self.request("POST")[0], 405)
        self.assertEqual(self.backend.stops, 0)
        self.assertEqual(self.lifecycle.status("operation-1")[1], "active")
        self.assertEqual(self.request("DELETE")[0], 200)
        self.assertEqual(self.backend.stops, 1)

    def test_revoke_during_slow_create_compensates_without_late_admission(self):
        allocated = threading.Event()
        release = threading.Event()
        original_allocate = self.backend.allocate

        def slow_allocate(operation_id, generation):
            allocated.set()
            if not release.wait(3):
                raise RuntimeError("test CREATE remained blocked")
            return original_allocate(operation_id, generation)

        self.backend.allocate = slow_allocate
        replies = []

        def create():
            replies.append(self.request("PUT"))

        caller = threading.Thread(target=create)
        caller.start()
        try:
            self.assertTrue(allocated.wait(3), "CREATE must reach the backend")
            status, pending = self.request("DELETE")
            self.assertEqual(status, 202, "pending CREATE is not confirmed STOP")
            self.assertIs(pending["stop_confirmed"], False)
            self.assertEqual(self.lifecycle.status("operation-1")[1], "stopping")
        finally:
            release.set()
            caller.join(timeout=4)

        self.assertFalse(caller.is_alive(), "late CREATE must finish compensation")
        self.assertEqual(len(replies), 1)
        self.assertEqual(replies[0][0], 409, "revoked CREATE must never publish a session")
        self.assertEqual(self.backend.allocations, 1)
        self.assertEqual(self.backend.stops, 1, "late allocation needs real compensating STOP")
        self.assertEqual(self.lifecycle.status("operation-1"), (1, "stopped", "media-1"))
        self.assertEqual(self.request("PUT")[0], 409)

    def test_cancelled_create_failure_remains_uncertain_without_sender_receipt(self):
        entered = threading.Event()
        release = threading.Event()

        def failed_allocate(operation_id, generation):
            entered.set()
            if not release.wait(3):
                raise RuntimeError("CREATE stalled")
            raise RuntimeError("provider CREATE timed out; sender identity unknown")

        self.backend.allocate = failed_allocate
        results = []

        def request_create():
            results.append(self.request("PUT"))

        caller = threading.Thread(target=request_create)
        caller.start()
        try:
            self.assertTrue(entered.wait(3), "CREATE must reach provider")
            status, result = self.request("DELETE")
            self.assertEqual(status, 202)
            self.assertIs(result["stop_confirmed"], False)
        finally:
            release.set()
            caller.join(timeout=4)

        self.assertFalse(caller.is_alive())
        self.assertEqual(len(results), 1)
        self.assertEqual(results[0][0], 409)
        self.assertEqual(
            self.lifecycle.status("operation-1"), (1, "uncertain", None)
        )
        self.assertEqual(self.request("PUT")[0], 409)
        self.assertEqual(self.backend.stops, 0)

    def test_hung_stop_returns_202_then_late_both_sender_receipt(self):
        self.assertEqual(self.request("PUT")[0], 200)
        entered = threading.Event()
        release = threading.Event()
        original_stop = self.backend.stop

        def blocked_stop(operation_id, session_id, generation):
            entered.set()
            if not release.wait(4):
                raise TimeoutError("simulated hung STOP")
            return original_stop(operation_id, session_id, generation)

        self.backend.stop = blocked_stop
        replies = []

        def revoke():
            replies.append(self.request("DELETE"))

        caller = threading.Thread(target=revoke)
        caller.start()
        try:
            self.assertTrue(entered.wait(2), "DELETE must reach the real STOP boundary")
            caller.join(timeout=2.5)
            self.assertFalse(caller.is_alive(), "HTTP DELETE must have a bounded wait")
            self.assertEqual(replies[0][0], 202)
            self.assertIs(replies[0][1]["stop_confirmed"], False)
            self.assertEqual(self.lifecycle.status("operation-1")[1], "stopping")
            # A second DELETE cannot launch another STOP behind a hung sender.
            self.assertEqual(self.request("DELETE")[0], 202)
            self.assertEqual(self.backend.stops, 0)
            self.assertEqual(self.request("PUT")[0], 409)
        finally:
            release.set()
            caller.join(timeout=2)
        deadline = time.monotonic() + 3
        while self.lifecycle.status("operation-1")[1] != "stopped" and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertEqual(self.lifecycle.status("operation-1"), (1, "stopped", "media-1"))
        self.assertEqual(self.backend.stops, 1)
        status, receipt = self.request("DELETE")
        self.assertEqual(status, 200)
        self.assertIs(receipt["audio_stopped"], True)
        self.assertIs(receipt["video_stopped"], True)
        self.assertEqual(self.backend.stops, 1)

    def test_late_stop_failure_remains_uncertain_until_restart_reconcile(self):
        self.assertEqual(self.request("PUT")[0], 200)
        entered = threading.Event()
        release = threading.Event()
        original_stop = self.backend.stop

        def failed_stop(operation_id, session_id, generation):
            entered.set()
            if not release.wait(4):
                raise TimeoutError("STOP timed out")
            raise ConnectionError("STOP receipt lost")

        self.backend.stop = failed_stop
        replies = []
        caller = threading.Thread(target=lambda: replies.append(self.request("DELETE")))
        caller.start()
        try:
            self.assertTrue(entered.wait(2))
            caller.join(timeout=2.5)
            self.assertFalse(caller.is_alive())
            self.assertEqual(replies[0][0], 202)
            self.assertIs(replies[0][1]["stop_confirmed"], False)
        finally:
            release.set()
            caller.join(timeout=2)
        deadline = time.monotonic() + 3
        while self.lifecycle.status("operation-1")[1] != "uncertain" and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertEqual(self.lifecycle.status("operation-1"), (1, "uncertain", "media-1"))
        self.assertEqual(self.request("PUT")[0], 409)
        self.backend.stop = original_stop
        restarted = MediaLifecycle(Path(self.temp.name) / "ledger.sqlite", self.backend)
        restarted.quarantine_after_restart()
        restarted.reconcile()
        self.assertEqual(restarted.status("operation-1"), (1, "stopped", "media-1"))
        self.assertEqual(self.backend.stops, 1)

    def _assert_signaling_delivery_serializes_remote_stop(self, suffix, payload, marker):
        """No SDP/acceptance may reach a browser after confirmed STOP.

        Pause the *actual HTTP write* after the provider call, then revoke
        from another HTTP worker. This catches the last-check-to-send race
        which pre/post provider authorization checks alone cannot prevent.
        """
        self.assertEqual(self.request("PUT")[0], 200)
        entered_write = threading.Event()
        release_write = threading.Event()
        original_send = self.server.RequestHandlerClass._send_fenced

        def paused_send(handler, status, body):
            if status == 200 and marker in body:
                entered_write.set()
                if not release_write.wait(3):
                    raise RuntimeError("test signaling write was never released")
            return original_send(handler, status, body)

        self.server.RequestHandlerClass._send_fenced = paused_send
        responses = []
        revoke_responses = []
        entered_cancel = threading.Event()
        original_cancel = self.lifecycle.cancel

        def observed_cancel(*args, **kwargs):
            entered_cancel.set()
            return original_cancel(*args, **kwargs)

        self.lifecycle.cancel = observed_cancel

        def signaling():
            request = Request(
                self.url + suffix, method="POST" if payload else "GET",
                data=json.dumps(payload).encode() if payload else None,
                headers={"Authorization": "Bearer test-secret", "Content-Type": "application/json"},
            )
            try:
                with urlopen(request, timeout=3) as response:
                    responses.append((response.status, json.load(response)))
            except HTTPError as error:
                responses.append((error.code, json.load(error)))

        sender = threading.Thread(target=signaling)
        revoke = threading.Thread(target=lambda: revoke_responses.append(self.request("DELETE")))
        sender.start()
        try:
            self.assertTrue(entered_write.wait(2), "signaling must reach the final HTTP write")
            revoke.start()
            self.assertTrue(entered_cancel.wait(2), "DELETE must reach durable CANCEL")
            time.sleep(0.08)
            self.assertFalse(revoke_responses, "STOP overtook an in-flight SDP delivery")
            self.assertEqual(self.backend.stops, 0, "remote sender stopped before SDP delivery")
        finally:
            release_write.set()
            sender.join(timeout=3)
            if revoke.ident is not None:
                revoke.join(timeout=3)
        self.assertFalse(sender.is_alive())
        self.assertFalse(revoke.is_alive())
        self.assertEqual(responses[0][0], 200)
        self.assertEqual(revoke_responses[0][0], 200)
        self.assertIs(revoke_responses[0][1]["audio_stopped"], True)
        self.assertIs(revoke_responses[0][1]["video_stopped"], True)
        self.assertEqual(self.lifecycle.status("operation-1"), (1, "stopped", "media-1"))

    def test_offer_delivery_is_serialized_with_both_sender_stop(self):
        self.backend.offer = lambda operation_id, session_id, generation: {
            "type": "offer", "sdp": "v=0 verified-offer",
        }
        self._assert_signaling_delivery_serializes_remote_stop("/offer", None, "sdp")

    def test_answer_acceptance_is_serialized_with_both_sender_stop(self):
        self.backend.answer = lambda operation_id, session_id, generation, sdp: None
        self._assert_signaling_delivery_serializes_remote_stop(
            "/answer", {"type": "answer", "sdp": "v=0 browser-answer"}, "accepted",
        )

    def test_provider_offer_cannot_replace_authoritative_sender_identity(self):
        self.backend.offer = lambda operation_id, session_id, generation: {
            "type": "offer", "sdp": "v=0 valid-offer",
            "session_id": "forged-sender", "operation_id": "forged-operation",
            "audio_stopped": True, "video_stopped": True,
        }
        self.assertEqual(self.request("PUT")[0], 200)
        request = Request(self.url + "/offer", headers={"Authorization": "Bearer test-secret"})
        with urlopen(request, timeout=3) as response:
            offer = json.load(response)
        self.assertEqual(offer["session_id"], "media-1")
        self.assertEqual(offer["operation_id"], "operation-1")
        self.assertNotIn("audio_stopped", offer)
        self.assertNotIn("video_stopped", offer)
        self.assertEqual(self.request("DELETE")[0], 200)

    def test_invalid_oversized_offer_revokes_remote_sender_before_error(self):
        self.backend.offer = lambda operation_id, session_id, generation: {
            "type": "offer", "sdp": "v=0 " + "x" * 65536,
        }
        self.assertEqual(self.request("PUT")[0], 200)
        request = Request(self.url + "/offer", headers={"Authorization": "Bearer test-secret"})
        with self.assertRaises(HTTPError) as rejected:
            urlopen(request, timeout=3)
        self.assertEqual(rejected.exception.code, 409)
        self.assertEqual(self.backend.stops, 1)
        self.assertEqual(self.lifecycle.status("operation-1"), (1, "stopped", "media-1"))
        self.assertEqual(self.request("PUT")[0], 409)

    def test_failed_offer_socket_delivery_compensates_both_remote_senders(self):
        self.backend.offer = lambda operation_id, session_id, generation: {
            "type": "offer", "sdp": "v=0 undeliverable-offer",
        }
        self.assertEqual(self.request("PUT")[0], 200)

        def broken_write(handler, status, body):
            if status == 200 and "sdp" in body:
                raise BrokenPipeError("simulated disconnected visitor")
            return original_write(handler, status, body)

        original_write = self.server.RequestHandlerClass._send_fenced
        self.server.RequestHandlerClass._send_fenced = broken_write
        request = Request(self.url + "/offer", headers={"Authorization": "Bearer test-secret"})
        from http.client import RemoteDisconnected
        from urllib.error import URLError
        with self.assertRaises((RemoteDisconnected, URLError)):
            urlopen(request, timeout=3)
        self.assertEqual(self.backend.stops, 1)
        self.assertEqual(self.lifecycle.status("operation-1"), (1, "stopped", "media-1"))
        self.assertEqual(self.request("PUT")[0], 409)

    def test_failed_answer_with_hung_remote_stop_returns_bounded_and_recovers(self):
        self.assertEqual(self.request("PUT")[0], 200)
        entered_stop = threading.Event()
        release_stop = threading.Event()
        original_stop = self.backend.stop

        def failed_answer(operation_id, session_id, generation, sdp):
            raise TimeoutError("provider accepted SDP but response timed out")

        def blocked_stop(operation_id, session_id, generation):
            entered_stop.set()
            if not release_stop.wait(4):
                raise TimeoutError("test STOP never released")
            return original_stop(operation_id, session_id, generation)

        self.backend.answer = failed_answer
        self.backend.stop = blocked_stop
        request = Request(
            self.url + "/answer", method="POST",
            data=json.dumps({"type": "answer", "sdp": "v=0 stalled-answer"}).encode(),
            headers={"Authorization": "Bearer test-secret", "Content-Type": "application/json"},
        )
        started = time.monotonic()
        try:
            with self.assertRaises(HTTPError) as rejected:
                urlopen(request, timeout=3)
            self.assertEqual(rejected.exception.code, 409)
            self.assertIs(json.load(rejected.exception)["stop_confirmed"], False)
            self.assertTrue(entered_stop.wait(2))
            self.assertLess(time.monotonic() - started, 2.5)
            self.assertEqual(self.lifecycle.status("operation-1")[1], "stopping")
            self.assertEqual(self.request("PUT")[0], 409)
        finally:
            release_stop.set()
        deadline = time.monotonic() + 3
        while self.lifecycle.status("operation-1")[1] != "stopped" and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertEqual(self.lifecycle.status("operation-1"), (1, "stopped", "media-1"))
        self.assertEqual(self.backend.stops, 1)

    def test_failed_answer_socket_delivery_compensates_both_remote_senders(self):
        self.backend.answer = lambda operation_id, session_id, generation, sdp: None
        self.assertEqual(self.request("PUT")[0], 200)

        def broken_write(handler, status, body):
            if status == 200 and body.get("accepted"):
                raise BrokenPipeError("simulated disconnected visitor")
            return original_write(handler, status, body)

        original_write = self.server.RequestHandlerClass._send_fenced
        self.server.RequestHandlerClass._send_fenced = broken_write
        request = Request(
            self.url + "/answer", method="POST",
            data=json.dumps({"type": "answer", "sdp": "v=0 browser-answer"}).encode(),
            headers={"Authorization": "Bearer test-secret", "Content-Type": "application/json"},
        )
        from http.client import RemoteDisconnected
        from urllib.error import URLError
        with self.assertRaises((RemoteDisconnected, URLError)):
            urlopen(request, timeout=3)
        self.assertEqual(self.backend.stops, 1)
        self.assertEqual(self.lifecycle.status("operation-1"), (1, "stopped", "media-1"))
        self.assertEqual(self.request("PUT")[0], 409)

    def test_offer_revoke_wins_during_slow_provider_without_late_sdp(self):
        self.assertEqual(self.request("PUT")[0], 200)
        entered = threading.Event()
        release = threading.Event()

        def slow_offer(operation_id, session_id, generation):
            entered.set()
            if not release.wait(3):
                raise TimeoutError("test offer stalled")
            return {"type": "offer", "sdp": "v=0 late-offer"}

        self.backend.offer = slow_offer
        replies = []

        def request_offer():
            request = Request(self.url + "/offer", headers={"Authorization": "Bearer test-secret"})
            try:
                with urlopen(request, timeout=3) as response:
                    replies.append((response.status, json.load(response)))
            except HTTPError as error:
                replies.append((error.code, json.load(error)))

        sender = threading.Thread(target=request_offer)
        sender.start()
        try:
            self.assertTrue(entered.wait(2))
            stopped_status, receipt = self.request("DELETE")
            self.assertEqual(stopped_status, 200)
            self.assertIs(receipt["audio_stopped"], True)
            self.assertIs(receipt["video_stopped"], True)
        finally:
            release.set()
            sender.join(timeout=3)
        self.assertFalse(sender.is_alive())
        self.assertEqual(replies[0][0], 410, "revoked offer leaked after remote STOP")
        self.assertEqual(self.lifecycle.status("operation-1"), (1, "stopped", "media-1"))

    def test_precreate_cancel_is_tombstone_not_fake_stop_receipt(self):
        status, body = self.request("DELETE")
        self.assertEqual(status, 202)
        self.assertIs(body["stop_confirmed"], False)
        self.assertEqual(self.request("PUT")[0], 409)
        self.assertEqual(self.backend.allocations, 0)


if __name__ == "__main__":
    unittest.main()
