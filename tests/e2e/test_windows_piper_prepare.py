"""Offline regression tests for pinned, idempotent Piper first-use installation."""
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[2] / "scripts" / "windows_piper_prepare.py"
SPEC = importlib.util.spec_from_file_location("windows_piper_prepare", SCRIPT)
bootstrap = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bootstrap)


class Response(io.BytesIO):
    url = "https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0/file"


class PiperFirstUseContract(unittest.TestCase):
    def test_config_rejects_wrong_language_and_sample_rate(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "voice.onnx.json"
            data = {
                "language": {"code": "ru_RU"},
                "audio": {"sample_rate": 22050},
                "dataset": "denis",
                "num_speakers": 1,
            }
            path.write_text(json.dumps(data), encoding="utf-8")
            self.assertTrue(bootstrap.config_is_valid(path))
            data["language"]["code"] = "en_US"
            path.write_text(json.dumps(data), encoding="utf-8")
            self.assertFalse(bootstrap.config_is_valid(path))
            data["language"]["code"] = "ru_RU"
            data["audio"]["sample_rate"] = 44100
            path.write_text(json.dumps(data), encoding="utf-8")
            self.assertFalse(bootstrap.config_is_valid(path))

    def test_cdn_redirect_matches_actual_official_model_host(self):
        self.assertTrue(bootstrap.is_trusted_voice_redirect(
            "https://us.aws.cdn.hf.co/voice/filename"
        ))
        self.assertTrue(bootstrap.is_trusted_voice_redirect(
            "https://cas-bridge.xethub.hf.co/model"
        ))
        for bad in (
            "http://us.aws.cdn.hf.co/model",
            "https://us.aws.cdn.hf.co.attacker.test/model",
            "https://evil.test/model",
            "https://username:password@us.aws.cdn.hf.co/model",
            "https://us.aws.cdn.hf.co:444/model",
        ):
            self.assertFalse(bootstrap.is_trusted_voice_redirect(bad), bad)

    def test_untrusted_host_fails_closed_without_partial_file(self):
        with tempfile.TemporaryDirectory() as root:
            output = Path(root) / "model.onnx"
            class Untrusted(Response):
                url = "https://other.example/model.onnx"
            with patch.object(bootstrap, "urlopen", return_value=Untrusted(b"payload")):
                with self.assertRaises(RuntimeError):
                    bootstrap.download_checked(
                        "https://huggingface.co/model", output, 500,
                        lambda path: path.read_bytes() == b"payload",
                    )
            self.assertFalse(output.exists())
            self.assertFalse(Path(str(output) + ".partial").exists())

    def test_download_rejects_integrity_error_and_never_overwrites_cached_voice(self):
        with tempfile.TemporaryDirectory() as root:
            output = Path(root) / "model.onnx"
            output.write_bytes(b"known-good")
            with patch.object(bootstrap, "urlopen", return_value=Response(b"bad-weights")):
                with self.assertRaises(RuntimeError):
                    bootstrap.download_checked(
                        "https://huggingface.co/model", output, 100,
                        lambda _: False,
                    )
            self.assertEqual(output.read_bytes(), b"known-good")
            self.assertFalse(Path(str(output) + ".partial").exists())

    def test_download_rejects_oversized_body(self):
        with tempfile.TemporaryDirectory() as root:
            output = Path(root) / "model.onnx"
            with patch.object(bootstrap, "urlopen", return_value=Response(bytes(25))):
                with self.assertRaises(RuntimeError):
                    bootstrap.download_checked(
                        "https://huggingface.co/model", output, 10, lambda _: True,
                    )
            self.assertFalse(output.exists())

    def test_valid_cached_voice_never_contacts_network(self):
        with tempfile.TemporaryDirectory() as root:
            voice = Path(root) / "voices" / (bootstrap.VOICE + ".onnx")
            voice.parent.mkdir()
            voice.write_bytes(b"cached")
            Path(str(voice) + ".json").write_text("{}", encoding="utf-8")
            with patch.object(bootstrap, "model_is_verified", return_value=True), \
                 patch.object(bootstrap, "config_is_valid", return_value=True), \
                 patch.object(bootstrap, "download_checked") as download:
                result = bootstrap.provision_voice(Path(root))
            self.assertEqual(result, voice)
            download.assert_not_called()

    def test_pinned_voice_and_package_versions(self):
        self.assertEqual(bootstrap.VOICE, "ru_RU-denis-medium")
        self.assertEqual(bootstrap.PIPER_VERSION, "1.3.0")
        self.assertEqual(len(bootstrap.MODEL_SHA256), 64)
        self.assertIn("/v1.0.0/", bootstrap.VOICE_URL)
        self.assertTrue(bootstrap.VOICE_URL.startswith("https://huggingface.co/"))


if __name__ == "__main__":
    unittest.main()
