"""Protect current server-owned playback receipt semantics from stale branch merges.

Server-owned speech has no browser command, but may still produce an
authoritative playback_done event. The browser must account for that receipt
without resending the server-owned command or fabricating completion.
"""
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FILES = (
    ROOT / "crates/vpr-owner-lab/ui/src/app.ts",
    ROOT / "crates/vpr-owner-lab/ui/dist/app.js",
)


class ServerOwnedPlaybackContract(unittest.TestCase):
    def test_server_owned_segments_expect_provider_receipt_without_browser_dispatch(self):
        for path in FILES:
            with self.subTest(path=path.name):
                text = path.read_text(encoding="utf-8")
                begin = text.index("const scheduleSegmentDelivery")
                end = text.index("let commandSent = false", begin)
                schedule = text[begin:end]
                count = schedule.index("providerPlaybackExpectedCount += 1")
                command = schedule.index("const command = segment.client_command")
                guard = schedule.index("if (!command)")
                observation = schedule.index("serverDeliveredSpeechObserved = true")
                self.assertLess(count, guard)
                self.assertLess(command, guard)
                self.assertLess(guard, observation)
                self.assertIn("return;", schedule[guard:])
                self.assertIn("syncRt0PlaybackPending(voiceForSegment)", schedule[:guard])
                self.assertNotIn("dispatchClientCommand(", schedule[:guard])

    def test_authoritative_completion_is_required_to_finalize_playback(self):
        for path in FILES:
            with self.subTest(path=path.name):
                source = path.read_text(encoding="utf-8")
                self.assertIn("providerPlaybackDoneCount += 1", source)
                self.assertIn("voice.providerPlaybackExpectedCount === 0", source)
                self.assertIn(
                    "voice.providerPlaybackDoneCount !== voice.providerPlaybackExpectedCount",
                    source,
                )
                self.assertIn("voice.responseComplete = true", source)


if __name__ == "__main__":
    unittest.main()
