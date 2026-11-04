"""Regression fence: server-owned speech must not fabricate browser playback receipts."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "crates/vpr-owner-lab/ui/src/app.ts"
DIST = ROOT / "crates/vpr-owner-lab/ui/dist/app.js"


class CommandlessPlaybackAccounting(unittest.TestCase):
    def test_source_and_browser_artifact_gate_before_increment(self):
        for path in (SRC, DIST):
            with self.subTest(path=path.name):
                source = path.read_text(encoding="utf-8")
                start = source.index("const scheduleSegmentDelivery")
                stop = source.index("let commandSent = false", start)
                block = source[start:stop]
                command = block.index("const command = segment.client_command")
                guard = block.index("if (!command)")
                expected = block.index("providerPlaybackExpectedCount += 1")
                self.assertLess(command, guard)
                self.assertLess(guard, expected)
                # Server-owned delivery stays observed; it is not a dropped turn.
                self.assertIn("serverDeliveredSpeechObserved = true", block[guard:expected])
                self.assertIn("return;", block[guard:expected])
                # Existing audio-fencing behavior survives for both delivery routes.
                self.assertGreaterEqual(block.count("setLocalAudioMuted(false)"), 2)
                self.assertIn("syncRt0PlaybackPending(voiceForSegment)", block[expected:])

if __name__ == "__main__":
    unittest.main()
