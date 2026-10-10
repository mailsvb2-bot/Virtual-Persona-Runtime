"""Hermetic security/contract tests for the optional MetroTrance local speech bridge."""
import http.client
import importlib.util
import json
import threading
import unittest
from http.server import ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("metrotrance_tts_bridge", ROOT / "scripts" / "metrotrance_tts_bridge.py")
module = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(module)

TOKEN = "local-secret-for-contract-testing-only"
WAV = b"RIFF" + bytes(4) + b"WAVE" + bytes(24)


class SpeechBridgeContract(unittest.TestCase):
    def setUp(self):
        self.phrases = []
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), module.make_handler(TOKEN, lambda text: self._voice(text)))
        self.worker = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.worker.start()

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.worker.join(timeout=2)

    def _voice(self, text):
        self.phrases.append(text)
        return WAV

    def post(self, path="/v1/audio/speech", payload=None, token=TOKEN):
        connection = http.client.HTTPConnection("127.0.0.1", self.server.server_port, timeout=3)
        data = json.dumps(payload if payload is not None else {"input": "Привет", "response_format": "wav"}).encode()
        connection.request("POST", path, data, {"Content-Type": "application/json", "Authorization": "Bearer " + token})
        response = connection.getresponse()
        result = (response.status, response.read(), response.getheader("Content-Type"))
        connection.close()
        return result

    def test_authorized_speech_returns_wav(self):
        status, wav, mime = self.post()
        self.assertEqual(status, 200)
        self.assertEqual(wav, WAV)
        self.assertEqual(mime, "audio/wav")
        self.assertEqual(self.phrases, ["Привет"])

    def test_authentication_fails_closed(self):
        status, _, _ = self.post(token="wrong")
        self.assertEqual(status, 401)
        self.assertEqual(self.phrases, [])

    def test_unexpected_route_does_not_synthesize(self):
        self.assertEqual(self.post(path="/admin")[0], 404)
        self.assertEqual(self.phrases, [])

    def test_invalid_requests_do_not_synthesize(self):
        self.assertEqual(self.post(payload={"input": ""})[0], 400)
        self.assertEqual(self.post(payload={"input": "x" * 4100})[0], 400)
        self.assertEqual(self.post(payload={"input": "x", "response_format": "mp3"})[0], 400)
        self.assertEqual(self.phrases, [])

    def test_non_object_json_is_client_error_not_provider_failure(self):
        for payload in (None, [], "hello", 5):
            status, _, _ = self.post(payload=payload)
            self.assertEqual(status, 400)
        self.assertEqual(self.phrases, [])

    def test_short_token_cannot_start_listener(self):
        with self.assertRaises(ValueError):
            module.make_handler("abc", lambda _: WAV)


if __name__ == "__main__":
    unittest.main()
