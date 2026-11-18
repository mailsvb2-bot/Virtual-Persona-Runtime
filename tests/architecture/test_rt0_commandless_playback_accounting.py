"""Lock server-owned playback expectation without fabricating browser receipts.

The provider may emit authoritative playback_done for server-owned output.
Count expected receipts for both paths, but dispatch browser commands only
when a client_command exists.
"""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
FILES = (
    ROOT / "crates/vpr-owner-lab/ui/src/app.ts",
    ROOT / "crates/vpr-owner-lab/ui/dist/app.js",
)


class ServerOwnedPlaybackAccounting(unittest.TestCase):
    def test_server_owned_output_expects_provider_receipt_without_browser_dispatch(self):
        for path in FILES:
            with self.subTest(path=str(path.relative_to(ROOT))):
                source = path.read_text(encoding="utf-8")
                start = source.index("const scheduleSegmentDelivery")
                stop = source.index("let commandSent = false", start)
                block = source[start:stop]
                expected = block.index("providerPlaybackExpectedCount += 1")
                guard = block.index("if (!command)")
                observed = block.index("serverDeliveredSpeechObserved = true")
                self.assertLess(expected, guard)
                self.assertLess(guard, observed)
                self.assertIn("syncRt0PlaybackPending(voiceForSegment)", block[:guard])
                self.assertIn("setLocalAudioMuted(false)", block[guard:])
                self.assertIn("return;", block[guard:])
                self.assertNotIn("dispatchClientCommand(", block[:stop-start])

    def test_provider_receipt_not_synthesized(self):
        for path in FILES:
            with self.subTest(path=path.name):
                source = path.read_text(encoding="utf-8")
                self.assertIn("providerPlaybackDoneCount += 1", source)
                self.assertIn("providerPlaybackDoneCount !== voice.providerPlaybackExpectedCount", source)


if __name__ == "__main__":
    unittest.main()
