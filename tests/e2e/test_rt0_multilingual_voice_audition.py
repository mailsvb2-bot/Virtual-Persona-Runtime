"""Fail-closed contracts for external-model audition. No PyTorch required."""
from __future__ import annotations

import hashlib
import io
import tempfile
from pathlib import Path
import sys
import unittest
from unittest.mock import patch
import wave

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
import rt0_multilingual_voice_audition as v


class VoiceAuditionContract(unittest.TestCase):
    def test_no_session_input_or_provider_tokens(self):
        self.assertEqual(v.ENGINES, ("chatterbox-v3", "qwen-base", "qwen-design"))
        self.assertIn("Russian", v.DESIGN_INSTRUCTION)
        self.assertIn("Здравствуйте", v.AUDITION_TEXT)
        self.assertTrue(v.QWEN_BASE.startswith("Qwen/Qwen3-TTS-"))
        self.assertTrue(v.QWEN_DESIGN.startswith("Qwen/Qwen3-TTS-"))

    def test_reference_needs_exact_sha_and_pcm_format(self):
        with tempfile.TemporaryDirectory() as d:
            ref = Path(d) / "synthetic.wav"
            with wave.open(str(ref), "wb") as out:
                out.setnchannels(1)
                out.setsampwidth(2)
                out.setframerate(24000)
                out.writeframes(b"\0\0" * (24000 * 3))
            with self.assertRaises(ValueError):
                v.validate_reference(ref)
            good_hash = hashlib.sha256(ref.read_bytes()).hexdigest()
            with patch.object(v, "TEST_REFERENCE_SHA256", good_hash):
                v.validate_reference(ref)
                ref.write_bytes(b"corrupted")
                with self.assertRaises(ValueError):
                    v.validate_reference(ref)

    def test_no_invalid_ref_or_backend_can_start_remote_inference(self):
        with self.assertRaises(ValueError):
            v.generate("other-provider", None)
        # Qwen VoiceDesign is the only mode without any cloned speaker.
        # Imports remain lazy; CI contract never downloads third-party models.
        self.assertTrue("native speaker of Russian" in v.DESIGN_INSTRUCTION)
        self.assertEqual(v.MAX_PCM_BYTES, 4 * 1024 * 1024)
        self.assertLessEqual(len(v.AUDITION_TEXT), 300)


if __name__ == "__main__":
    unittest.main()
