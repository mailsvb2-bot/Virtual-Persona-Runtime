"""RT0 #258: revoke must preserve terminal evidence for its own voice generation.

The canonical replay fence prevents an old worker from modifying a new session.
The cancellation path must also finalize pending evidence before session export.
"""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
TERMINAL = ROOT / "crates/vpr-owner-lab/src/http_voice_terminal.rs"
SESSION = ROOT / "crates/vpr-owner-lab/src/http_session.rs"


class VoiceTerminalRevokeEvidenceContract(unittest.TestCase):
    def test_cancelled_worker_is_terminalized_only_for_same_generation(self):
        code = TERMINAL.read_text(encoding="utf-8")
        begin = code.index(") else {", code.index("begin_voice_terminal_replay("))
        end = code.index("    };", begin)
        denied = code[begin:end]
        self.assertIn("state.replay_source.lock()", denied)
        self.assertIn("session_sequence != 0", denied)
        self.assertIn("active_session_sequence.load(Ordering::Acquire) == session_sequence", denied)
        self.assertIn("session_end_requested.load(Ordering::Acquire)", denied)
        self.assertIn('fail_voice_request(request_sequence, "AUTH_REVOKED")', denied)
        self.assertLess(denied.index("state.replay_source.lock()"),
                        denied.index("fail_voice_request("))
        self.assertIn("VoiceStreamEvent::Failed", denied)
        self.assertNotIn("retain_completed_reply_locked", denied)

    def test_revoke_session_seals_after_voice_workers_quiesce(self):
        code = SESSION.read_text(encoding="utf-8")
        self.assertIn("state.voice_streams.wait_until_quiescent()", code)
        self.assertIn("state.evidence.lock().seal_session()", code)


if __name__ == "__main__":
    unittest.main()
