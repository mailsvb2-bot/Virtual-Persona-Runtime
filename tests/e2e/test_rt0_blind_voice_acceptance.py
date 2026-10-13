"""Real offline WAV input, blind-browser HTML, human attestations and rights gates.

No heavyweight TTS dependency, provider credential or private audio is needed.
"""
from __future__ import annotations

import hashlib
import json
import math
import struct
from pathlib import Path
import sys
import tempfile
import unittest
import wave
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
import rt0_blind_voice_acceptance as blind


class BlindVoiceAcceptanceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "original"
        self.package = self.root / "anonymous"
        self.private = self.root / "separate" / "mapping.json"
        self.source.mkdir()
        for index, name in enumerate(("qwen-design", "chatterbox-v3")):
            audio = self.source / f"{name}.wav"
            with wave.open(str(audio), "wb") as output:
                output.setnchannels(1)
                output.setsampwidth(2)
                output.setframerate(24000)
                samples = (int(8000 * math.sin(2 * math.pi * (180 + index * 65) * i / 24000))
                           for i in range(24000))
                output.writeframes(struct.pack("<24000h", *samples))
            metadata = {
                "engine": name,
                "audition_text_sha256": blind.TEXT_SHA256,
                "file": audio.name,
                "seconds": 1.0,
                "reference_type": "none" if name == "qwen-design" else "synthetic_mit_sample",
                "canonical_playback_proven": False,
            }
            (self.source / f"{name}.json").write_text(
                json.dumps(metadata), encoding="utf-8"
            )

    def prepare(self):
        return blind.prepare(self.source, self.package, self.private)

    def rights(self, licensed=True):
        candidates = {
            name: {
                "commercial_use_confirmed": bool(licensed),
                "therapeutic_context_confirmed": bool(licensed),
                "voice_reference_consent_confirmed": bool(licensed),
                "reviewer": "Reviewer acknowledgement, not independently verified",
                "evidence_url": "https://example.org/rights/review",
            } for name in ("qwen-design", "chatterbox-v3")
        }
        file = self.root / "rights.json"
        file.write_text(json.dumps({
            "schema_version": "rt0-voice-rights-review-1",
            "candidates": candidates,
        }), encoding="utf-8")
        return file

    def rating(self, index, score=5, attested=True, session=None):
        manifest = blind.read_json(self.private)
        file = self.root / f"human-{index}.json"
        file.write_text(json.dumps({
            "schema_version": "rt0-blind-listener-score-1",
            "session_id": session or manifest["session_id"],
            "human_listening_attested": attested,
            "reviewer_key": f"test-human-{index}",
            "ratings": [{
                "blind_label": item["blind_label"],
                "scores": {metric: score for metric in blind.SCORES},
                "notes": "",
            } for item in manifest["candidates"]],
        }), encoding="utf-8")
        return file

    def test_generates_fixed_text_anonymized_audio_and_separate_mapping(self):
        result = self.prepare()
        self.assertEqual(result["candidates"], 2)
        self.assertFalse(result["release_ready"])
        self.assertEqual(
            sorted(p.name for p in self.package.iterdir()),
            ["A.wav", "B.wav", "index.html"],
        )
        listener_html = (self.package / "index.html").read_text(encoding="utf-8")
        self.assertIn("Здравствуйте!", listener_html)
        self.assertIn("russian_nativeness", listener_html)
        for provider in ("qwen-design", "chatterbox-v3"):
            self.assertNotIn(provider, listener_html)
        mapping = blind.read_json(self.private)
        self.assertEqual({x["engine"] for x in mapping["candidates"]},
                         {"qwen-design", "chatterbox-v3"})
        for item in mapping["candidates"]:
            self.assertEqual(
                blind.sha256(self.package / f"{item['blind_label']}.wav"),
                item["audio_sha256"],
            )

    def test_real_second_attestation_and_rights_allow_trial_but_never_release(self):
        self.prepare()
        r1, r2 = self.rating(1), self.rating(2)
        decision = blind.assess(self.private, self.package, [r1, r2], self.rights())
        self.assertIn(decision["selected_for_live_trial"],
                      {"qwen-design", "chatterbox-v3"})
        self.assertFalse(decision["release_ready"])
        self.assertFalse(decision["human_identity_independently_verified"])
        self.assertFalse(decision["canonical_audio_playout_proven"])
        self.assertFalse(decision["avatar_voice_appearance_fit_proven"])

    def test_no_permissions_or_poor_russian_pronunciation_means_no_selection(self):
        self.prepare()
        a, b = self.rating(1), self.rating(2)
        denied = blind.assess(self.private, self.package, [a, b], self.rights(False))
        self.assertIsNone(denied["selected_for_live_trial"])
        a, b = self.rating(3, 2), self.rating(4, 2)
        poor = blind.assess(self.private, self.package, [a, b], self.rights())
        self.assertIsNone(poor["selected_for_live_trial"])

    def test_silent_dc_only_and_severely_clipped_wav_are_not_human_voice_candidates(self):
        audio = self.source / "qwen-design.wav"
        with wave.open(str(audio), "wb") as stream:
            stream.setnchannels(1)
            stream.setsampwidth(2)
            stream.setframerate(24000)
            stream.writeframes(bytes(48000))
        with self.assertRaisesRegex(ValueError, "silent, DC-only"):
            self.prepare()
        with wave.open(str(audio), "wb") as stream:
            stream.setnchannels(1)
            stream.setsampwidth(2)
            stream.setframerate(24000)
            stream.writeframes(struct.pack("<24000h", *([1000] * 24000)))
        with self.assertRaisesRegex(ValueError, "silent, DC-only"):
            self.prepare()
        with wave.open(str(audio), "wb") as stream:
            stream.setnchannels(1)
            stream.setsampwidth(2)
            stream.setframerate(24000)
            stream.writeframes(struct.pack("<24000h", *([32767, -32768] * 12000)))
        with self.assertRaisesRegex(ValueError, "severely clipped"):
            self.prepare()

    def test_rejects_single_reviewer_cross_session_and_missing_attestation(self):
        self.prepare()
        a = self.rating(1)
        with self.assertRaises(ValueError):
            blind.assess(self.private, self.package, [a], self.rights())
        b = self.rating(2, session="other")
        with self.assertRaises(ValueError):
            blind.assess(self.private, self.package, [a, b], self.rights())
        b = self.rating(3, attested=False)
        with self.assertRaises(ValueError):
            blind.assess(self.private, self.package, [a, b], self.rights())

    def test_repeated_review_file_or_same_listener_pseudonym_cannot_count_twice(self):
        self.prepare()
        a = self.rating(1)
        with self.assertRaisesRegex(ValueError, "duplicate listening file"):
            blind.assess(self.private, self.package, [a, a], self.rights())
        b = self.rating(2)
        item = blind.read_json(b)
        item["reviewer_key"] = "test-human-1"
        b.write_text(json.dumps(item), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "duplicate listener identity"):
            blind.assess(self.private, self.package, [a, b], self.rights())

    def test_tampered_audio_mapping_and_mismatched_text_fail_closed(self):
        self.prepare()
        a, b = self.rating(1), self.rating(2)
        first = self.package / "A.wav"
        first.write_bytes(first.read_bytes() + b"tamper")
        with self.assertRaisesRegex(ValueError, "no longer matches"):
            blind.assess(self.private, self.package, [a, b], self.rights())

        self.package = self.root / "new-anonymous"
        self.private = self.root / "new-private" / "mapping.json"
        metadata = self.source / "qwen-design.json"
        record = json.loads(metadata.read_text(encoding="utf-8"))
        record["audition_text_sha256"] = hashlib.sha256(b"different").hexdigest()
        metadata.write_text(json.dumps(record), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "fixed-Russian-text"):
            self.prepare()

    def test_refuses_to_overwrite_existing_or_expose_private_mapping(self):
        self.prepare()
        with self.assertRaises(ValueError):
            self.prepare()
        with tempfile.TemporaryDirectory() as alternate:
            package = Path(alternate) / "pkg"
            with self.assertRaisesRegex(ValueError, "outside listener"):
                blind.prepare(self.source, package, package / "mapping.json")

    def test_no_fabricated_scores_and_broken_rights_declarations(self):
        self.prepare()
        a, b = self.rating(1), self.rating(2)
        raw = blind.read_json(b)
        raw["ratings"][0]["scores"]["russian_nativeness"] = 5.5
        b.write_text(json.dumps(raw), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "integral"):
            blind.assess(self.private, self.package, [a, b], self.rights())
        b = self.rating(3)
        r = self.rights()
        rights = blind.read_json(r)
        rights["candidates"]["qwen-design"]["commercial_use_confirmed"] = False
        r.write_text(json.dumps(rights), encoding="utf-8")
        decision = blind.assess(self.private, self.package, [a, b], r)
        rejected = [x for x in decision["results"] if x["engine"] == "qwen-design"][0]
        self.assertFalse(rejected["eligible_for_live_trial"])


if __name__ == "__main__":
    unittest.main()
