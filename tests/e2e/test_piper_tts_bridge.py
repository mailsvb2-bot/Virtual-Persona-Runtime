"""Piper bridge tests without model downloads or external credentials."""
import importlib.util
import io
import sys
import tempfile
import types
import unittest
import wave
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
spec = importlib.util.spec_from_file_location("piper_tts_bridge", ROOT / "scripts/piper_tts_bridge.py")
bridge = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bridge)


class PiperContract(unittest.TestCase):
    def test_requires_local_model_and_config(self):
        with tempfile.TemporaryDirectory() as d:
            model = Path(d) / "ru_RU-test.onnx"
            model.write_bytes(b"test")
            with self.assertRaises(ValueError):
                bridge.make_piper_synthesizer(str(model))


    def test_loopback_http_enforces_token_model_and_wav(self):
        import http.client
        import json
        import threading
        from http.server import ThreadingHTTPServer
        from metrotrance_tts_bridge import make_handler

        wav = io.BytesIO()
        with wave.open(wav, "wb") as sink:
            sink.setnchannels(1)
            sink.setsampwidth(2)
            sink.setframerate(22050)
            sink.writeframes(bytes(200))
        token = "piper-secret-token-for-e2e-tests"
        phrases = []
        def synthesize(phrase):
            phrases.append(phrase)
            return wav.getvalue()
        server = ThreadingHTTPServer(
            ("127.0.0.1", 0), make_handler(token, synthesize, provider_id="piper")
        )
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        def request(secret, model):
            client = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=3)
            try:
                client.request(
                    "POST", "/v1/audio/speech",
                    json.dumps({"input": "Привет", "model": model,
                                "voice": "local", "response_format": "wav"}).encode("utf-8"),
                    {"Authorization": "Bearer " + secret,
                     "Content-Type": "application/json"},
                )
                response = client.getresponse()
                return response.status, response.read()
            finally:
                client.close()
        try:
            self.assertEqual(request("wrong-token", "piper")[0], 401)
            self.assertEqual(request(token, "metrotrance-qwen")[0], 400)
            self.assertEqual(phrases, [])
            status, audio = request(token, "piper")
            self.assertEqual(status, 200)
            self.assertEqual(audio, wav.getvalue())
            self.assertEqual(phrases, ["Привет"])
        finally:
            server.shutdown()
            server.server_close()
            worker.join(timeout=2)

    def test_preset_synthesizes_pcm_wav(self):
        with tempfile.TemporaryDirectory() as d:
            model = Path(d) / "ru_RU-test.onnx"
            model.write_bytes(b"test")
            Path(str(model) + ".json").write_text("{}")
            class FakeVoice:
                @staticmethod
                def load(path):
                    self.assertEqual(path, str(model))
                    return FakeVoice()
                def synthesize_wav(self, text, out):
                    self.assertEqual(text, "Привет")
                    out.setnchannels(1)
                    out.setsampwidth(2)
                    out.setframerate(22050)
                    out.writeframes(b"\\0\\0" * 100)
            with patch.dict(sys.modules, {"piper": types.SimpleNamespace(PiperVoice=FakeVoice)}):
                audio = bridge.make_piper_synthesizer(str(model))("Привет")
            with wave.open(io.BytesIO(audio)) as wav:
                self.assertEqual(wav.getframerate(), 22050)
                self.assertEqual(wav.getsampwidth(), 2)


if __name__ == "__main__":
    unittest.main()
