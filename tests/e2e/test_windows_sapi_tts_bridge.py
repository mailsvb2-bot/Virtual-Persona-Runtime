"""RT0 native Windows voice: canonical loopback auth and Unicode contract."""
import base64
import http.client
from http.server import ThreadingHTTPServer
import io
import json
from pathlib import Path
import sys
import threading
import unittest
import wave

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from metrotrance_tts_bridge import make_handler, validate_echo_wav
import windows_sapi_tts_bridge as bridge


def wav_fixture():
    out = io.BytesIO()
    with wave.open(out, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(22050)
        w.writeframes(bytes(4400))
    return out.getvalue()


class WindowsSapiContract(unittest.TestCase):
    def test_fixed_script_uses_native_russian_female_and_safe_stdin(self):
        script = bridge._PS_SCRIPT
        self.assertIn('Microsoft Irina Desktop', script)
        self.assertIn('ru-RU', script)
        self.assertIn('Female', script)
        self.assertIn('FromBase64String', script)
        self.assertIn('SetOutputToWaveStream', script)
        self.assertNotIn('Invoke-WebRequest', script)

    def test_private_http_accepts_only_right_token_and_provider(self):
        observed = []
        wav = wav_fixture()
        def synth(text):
            observed.append(text)
            return wav
        token = "test-windows-russian-female-voice-private"
        server = ThreadingHTTPServer(
            ("127.0.0.1", 0),
            make_handler(token, synth, provider_id="windows-sapi"),
        )
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        def request(token_to_use, model):
            conn = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=3)
            try:
                conn.request(
                    "POST", "/v1/audio/speech",
                    json.dumps({"model": model, "voice": "local",
                                "response_format": "wav",
                                "input": "Здравствуйте, я рада вас видеть!"}, ensure_ascii=False)
                    .encode("utf-8"),
                    {"Authorization": "Bearer " + token_to_use,
                     "Content-Type": "application/json"},
                )
                response = conn.getresponse()
                return response.status, response.read()
            finally:
                conn.close()
        try:
            self.assertEqual(request("incorrect", "windows-sapi")[0], 401)
            self.assertEqual(request(token, "piper")[0], 400)
            self.assertEqual(observed, [])
            status, data = request(token, "windows-sapi")
            self.assertEqual(status, 200)
            self.assertEqual(data, wav)
            self.assertTrue(validate_echo_wav(data))
            self.assertEqual(observed, ["Здравствуйте, я рада вас видеть!"])
        finally:
            server.shutdown()
            server.server_close()
            worker.join(timeout=3)


if __name__ == "__main__":
    unittest.main()
