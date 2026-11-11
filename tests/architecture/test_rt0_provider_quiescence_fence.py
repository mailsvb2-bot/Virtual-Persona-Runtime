"""RT0 HTTP revoke/close must not report success while a provider still runs."""
from pathlib import Path
import unittest

SOURCE = (
    Path(__file__).resolve().parents[2]
    / "crates/vpr-owner-lab/src/http_session.rs"
)


def block(source: str, name: str, next_name: str) -> str:
    return source.split(f"fn {name}(", 1)[1].split(f"fn {next_name}(", 1)[0]


class ProviderEgressFenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = SOURCE.read_text(encoding="utf-8")

    def test_fence_requires_provider_drained_after_canonical_revoke(self):
        fence = block(self.source, "fence_authority", "fence_session_response")
        self.assertLess(
            fence.index("revoker\n        .revoke_authority()"),
            fence.index("revoker.await_provider_quiescence(Duration::ZERO)"),
        )
        self.assertIn(
            "Ok(stopped && revoker.await_provider_quiescence(Duration::ZERO))",
            fence,
        )
        self.assertNotIn("state.engine.lock()", fence)

    def test_deferred_cleanup_waits_for_real_provider_and_audio_drain(self):
        helper = block(self.source, "provider_egress_quiescent", "finish_after_quiescence")
        self.assertIn("state.session_revocation.lock().clone()", helper)
        self.assertIn("revoker.await_provider_quiescence(timeout)", helper)
        cleanup = block(self.source, "finish_after_quiescence", "end_session")
        self.assertIn("state.voice_streams.wait_until_quiescent()", cleanup)
        self.assertIn(
            "provider_egress_quiescent(state, Duration::from_millis(100))",
            cleanup,
        )
        self.assertIn("complete_deferred_session(state, close, expected_generation)", cleanup)
        self.assertLess(
            cleanup.index("provider_egress_quiescent(state, Duration::from_millis(100))"),
            cleanup.index("complete_deferred_session(state, close, expected_generation)"),
        )

    def test_close_fails_bounded_before_provider_quiescence(self):
        closing = self.source.split("fn end_session(", 1)[1]
        self.assertIn("|| !provider_egress_quiescent(state, Duration::ZERO)", closing)
        self.assertIn('error_response(504, "PROVIDER_TIMEOUT")', closing)
        self.assertLess(
            closing.index("provider_egress_quiescent(state, Duration::ZERO)"),
            closing.index("complete_deferred_session(state, close, expected_generation)?"),
        )


if __name__ == "__main__":
    unittest.main()
