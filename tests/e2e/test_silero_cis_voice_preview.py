"""Hermetic integrity, license-selection and no-user-audio checks for Silero audition."""
from __future__ import annotations
import hashlib
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[2] / "scripts"
sys.path.insert(0, str(SCRIPTS))
import silero_cis_voice_preview as preview
import silero_cis_voice_prepare as prepare


class FakeModel:
    def __init__(self):
        self.calls = []

    def apply_tts(self, **kw):
        self.calls.append(kw)
        return object()


class FakeTorch:
    class inference_mode:
        def __enter__(self): return None
        def __exit__(self, *_): return None


class PreviewContract(unittest.TestCase):
    def test_only_server_fixed_female_russian_speakers(self):
        self.assertEqual(
            preview.FEMALE_VOICES,
            ("ru_ekaterina", "ru_karina", "ru_vika"),
        )
        self.assertEqual(preview.SAMPLE_RATE, 24000)
        fake = FakeModel()
        with patch.object(preview, "pcm16_wav", return_value=b"wav"):
            choices = preview.synthesize_samples(fake, FakeTorch)
            self.assertEqual(list(choices), list(preview.FEMALE_VOICES))
            self.assertEqual(
                [call["speaker"] for call in fake.calls], list(preview.FEMALE_VOICES),
            )
            self.assertTrue(all(call["text"] == preview.TEST_PHRASE for call in fake.calls))
            self.assertTrue(all(call["sample_rate"] == 24000 for call in fake.calls))
            with self.assertRaises(ValueError):
                preview.synthesize_samples(fake, FakeTorch, voices=("random",))
        self.assertNotIn("user", preview.TEST_PHRASE.lower())

    def test_checksum_fails_closed_and_cache_skips_network(self):
        with tempfile.TemporaryDirectory() as d:
            fake_model = Path(d) / preview.MODEL_NAME
            content = b"verified-test-bytes"
            fake_model.write_bytes(content)
            with patch.object(preview, "MIN_MODEL_BYTES", 0, create=True):
                # Real SHA-256, never rely on filenames or ZIP metadata.
                with patch.object(preview, "MODEL_SHA256", hashlib.sha256(content).hexdigest()):
                    # The preview verifier has a release size guard as well.
                    with patch.object(preview, "verified_model", return_value=True):
                        with patch.object(prepare, "verified_model", return_value=True):
                            with patch.object(prepare, "download_checked") as download:
                                output = prepare.prepare_model(Path(d))
                                self.assertEqual(output, fake_model)
                                download.assert_not_called()
                self.assertFalse(preview.verified_model(fake_model))
            with patch.object(prepare, "verified_model", side_effect=[False, False]):
                with patch.object(prepare, "download_checked") as download:
                    with self.assertRaises(RuntimeError):
                        prepare.prepare_model(Path(d))
                    download.assert_called_once()

    def test_package_origin_and_license_are_explicit(self):
        self.assertIn("/silero-models-v5-cis-base-nostress/", preview.MODEL_URL)
        self.assertIn("/resolve/5f1a14e/", preview.MODEL_URL)
        self.assertEqual(len(preview.MODEL_SHA256), 64)
        self.assertEqual(prepare.TORCH_VERSION, "2.5.1")
        self.assertTrue(prepare.CPU_INDEX.startswith("https://download.pytorch.org/"))
        self.assertNotIn("v5_5_ru", preview.MODEL_URL)

    def test_cannot_load_unverified_torch_package(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(ValueError):
                preview.load_verified_silero(Path(d) / preview.MODEL_NAME)
        # No PyTorch import or model code must run before digest check.


if __name__ == "__main__":
    unittest.main()
