"""Protect the strict A/V evidence start boundary without fabricating RTP proof."""
from pathlib import Path
import unittest

APP = Path(__file__).resolve().parents[2] / "crates/vpr-owner-lab/ui/src/app.ts"

class AudioSyncStartContract(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.code = APP.read_text(encoding="utf-8")

    def test_start_from_observed_audio_not_backend_completion(self):
        start = self.code.index("const ensureAvSyncEvidence =")
        end = self.code.index("const ensurePlaybackCompletionEvidence =", start)
        body = self.code[start:end]
        self.assertIn("!voice.audioStarted", body)
        self.assertIn("voice.audioStartedElapsed === null", body)
        self.assertIn("voice.interrupted", body)
        self.assertNotIn("!voice.responseComplete", body)

    def test_sample_origin_is_strict_rtp_and_fail_closed(self):
        start = self.code.index("const collectAvSyncEvidence =")
        end = self.code.index("const ensureAvSyncEvidence =", start)
        body = self.code[start:end]
        self.assertIn('"/api/evidence/av-sync"', body)
        self.assertIn('"/api/evidence/av-sync-diagnostic"', body)
        self.assertIn("reading.reference", body)
        self.assertIn("reading.offsetMillis !== null", body)
        self.assertIn("AV_SYNC_SAMPLE_COUNT", body)

    def test_start_called_from_audio_detection(self):
        start = self.code.index('postMediaEvidence(\n          "audio_started"')
        end = self.code.index("voice.audioStartedEvidence = audioStartedEvidence;",start)
        self.assertIn("ensureAvSyncEvidence(voice)",self.code[start:end])

if __name__ == "__main__":
    unittest.main()
