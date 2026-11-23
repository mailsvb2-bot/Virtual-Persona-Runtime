"""RT0 #258: revoke must not wait for provider I/O behind the reply cache lock.

This source-level regression guards the lock boundary in the canonical HTTP path.
Backend/browser E2E separately covers authorized replay and refusal after revoke.
"""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
CONTROL = ROOT / "crates/vpr-owner-lab/src/http_client_control.rs"
SESSION = ROOT / "crates/vpr-owner-lab/src/http_session.rs"


class ResumeRevokeLockFence(unittest.TestCase):
    def test_resume_provider_io_does_not_hold_replay_cache_mutex(self):
        code = CONTROL.read_text(encoding="utf-8")
        start = code.index('"/api/avatar/resume-answer" =>')
        end = code.index('"/api/avatar/client-event" =>', start)
        route = code[start:end]
        self.assertIn("let suffix = {", route)
        self.assertIn("let source = state.replay_source.lock();", route)
        self.assertIn("let result = with_engine_result", route)
        self.assertLess(route.index("let suffix = {"),
                        route.index("let result = with_engine_result"))
        # The first lock ends at the scoped suffix expression. The second
        # lock must come only after remote provider preparation has returned.
        self.assertIn("\n                };\n                let result = with_engine_result", route)
        self.assertLess(route.index("let result = with_engine_result"),
                        route.index("let mut source = state.replay_source.lock();"))
        self.assertLess(route.index("let mut source = state.replay_source.lock();"),
                        route.rindex("reject_if_session_ending(state)?;"))
        self.assertIn("reply.request_sequence != body.request_sequence", route)
        self.assertIn("reply.resume_count >= MAX_RESUMES_PER_REPLY", route)

    def test_session_revoke_intent_remains_under_same_canonical_mutex(self):
        source = SESSION.read_text(encoding="utf-8")
        start = source.index("fn fence_authority(")
        end = source.index("fn ", start + 4)
        body = source[start:end]
        self.assertLess(body.index("state.replay_source.lock()"),
                        body.index("state.session_end_requested.store(true"))
        self.assertLess(body.index("state.session_end_requested.store(true"),
                        body.index("*replay = None"))


if __name__ == "__main__":
    unittest.main()
