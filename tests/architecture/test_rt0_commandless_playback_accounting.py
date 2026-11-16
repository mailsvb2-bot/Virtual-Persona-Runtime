"""RT0 server-owned delivery must not fabricate browser playback receipts.

Protect the production TypeScript source AND checked-in browser output. These
assertions supplement backend/browser end-to-end tests; they do not claim
provider-side playout or remote RTP STOP proof.
"""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
FILES = (
    ROOT / "crates/vpr-owner-lab/ui/src/app.ts",
    ROOT / "crates/vpr-owner-lab/ui/dist/app.js",
)


class CommandlessPlaybackAccounting(unittest.TestCase):
    def test_only_browser_commands_increment_expected_playback(self):
        for path in FILES:
            with self.subTest(path=str(path.relative_to(ROOT))):
                source = path.read_text(encoding="utf-8")
                start = source.index("const scheduleSegmentDelivery")
                stop = source.index("let commandSent = false", start)
                block = source[start:stop]
                command = block.index("const command = segment.client_command")
                guard = block.index("if (!command)")
                expected = block.index("providerPlaybackExpectedCount += 1")
                self.assertLess(command, guard)
                self.assertLess(guard, expected)
                server_branch = block[guard:expected]
                self.assertIn("serverDeliveredSpeechObserved = true", server_branch)
                self.assertIn("return;", server_branch)
                # Provider-owned speech is still authorized to unmute.
                self.assertIn("setLocalAudioMuted(false)", server_branch)
                # Browser-delivered speech retains pending-completion tracking.
                self.assertIn("syncRt0PlaybackPending(voiceForSegment)", block[expected:])
                self.assertGreaterEqual(block.count("setLocalAudioMuted(false)"), 2)


if __name__ == "__main__":
    unittest.main()
