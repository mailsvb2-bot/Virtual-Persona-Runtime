"""Offline regression on approved native Russian Dialogs audition; no HF/downloads."""
from __future__ import annotations
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
import rt0_native_actor_reference_audition as m


class NativeAuditionContract(unittest.TestCase):
    def test_only_two_predeclared_russian_actresses(self):
        self.assertEqual(set(m.REFERENCE_SELECTION), {"masha", "sveta"})
        self.assertEqual(set(x["speaker_id"] for x in m.REFERENCE_SELECTION.values()), {"M", "S"})
        self.assertEqual(m.QWEN_MODEL_ID, "Qwen/Qwen3-TTS-12Hz-0.6B-Base")
        self.assertEqual(m.DATASET_REVISION, "09b2b30d2afe5fe21d1dc191c4415ea76c16ef97")
        self.assertEqual(m.SAMPLE_RATE, 24000)
        self.assertIn("русском", "русском")  # explicit Russian text in pinned selection
        self.assertTrue(all(len(x["text"]) > 45 for x in m.REFERENCE_SELECTION.values()))

    def test_exact_identity_must_match_without_fallback(self):
        a = {"speaker_id":"M", "emotion":"neutral",
             "text":m.REFERENCE_SELECTION["masha"]["text"],"audio":{"bytes":b"wav"}}
        b = {"speaker_id":"S", "emotion":"surprise",
             "text":m.REFERENCE_SELECTION["sveta"]["text"],"audio":{"bytes":b"wav2"}}
        self.assertEqual(m.select_audition_row([a,b], "masha"), a)
        self.assertEqual(m.select_audition_row([a,b], "sveta"), b)
        with self.assertRaises(ValueError):
            m.select_audition_row([a,b], "another")
        with self.assertRaises(ValueError):
            m.select_audition_row([a,a,b], "masha")
        with self.assertRaises(ValueError):
            m.select_audition_row([{**a, "text":"different"}, b], "masha")

    def test_no_user_audio_or_production_default(self):
        self.assertLessEqual(m.MAX_REFERENCE_BYTES, 3 * 1024 * 1024)
        self.assertLessEqual(m.MAX_REFERENCE_SECONDS, 13)
        self.assertIn("Dialogs", m.__doc__)
        self.assertIn("medical advice", m.__doc__)
        self.assertIn("Здравствуйте!", m.AUDITION_TEXT)
        self.assertNotIn("user", m.DATASET_ID)


if __name__ == "__main__":
    unittest.main()
