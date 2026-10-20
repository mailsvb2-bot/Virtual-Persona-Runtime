"""Guard RT0 audio/video sampling against stale STOP and next-turn races."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "crates/vpr-owner-lab/ui/src/app.ts"
DIST = ROOT / "crates/vpr-owner-lab/ui/dist/app.js"

class RTPCollectionLifecycleTest(unittest.TestCase):
    def test_stale_sessions_and_interrupted_turns_never_upload_more_samples(self):
        for path in (SRC, DIST):
            with self.subTest(path=path):
                code = path.read_text(encoding="utf-8")
                begin = code.index("const collectAvSyncEvidence =")
                end = code.index("const ensureAvSyncEvidence =", begin)
                block = code[begin:end]
                self.assertIn("activeVoiceEvidence?.requestSequence === requestSequence", block)
                self.assertIn("!activeVoiceEvidence.interrupted", block)
                self.assertGreaterEqual(block.count("if (!isActiveRequest())"), 3)
                self.assertIn('"/api/evidence/av-sync"', block)
                self.assertIn('"/api/evidence/av-sync-diagnostic"', block)

    def test_older_collection_cannot_clear_newer_busy_flag(self):
        for path in (SRC, DIST):
            with self.subTest(path=path):
                code = path.read_text(encoding="utf-8")
                begin = code.index("const ensureAvSyncEvidence =")
                end = code.index("const ensurePlaybackCompletionEvidence =", begin)
                block = code[begin:end]
                self.assertIn("if (pendingAvSyncEvidence === collection) {", block)
                self.assertLess(block.index("if (pendingAvSyncEvidence === collection) {"),
                                block.index("avSyncCollectionPending = false;"))

if __name__ == "__main__":
    unittest.main()
