"""RT1 provider startup remains closed until enforceable provider cancellation exists."""
from pathlib import Path
import re
import unittest

SOURCE = (
    Path(__file__).resolve().parents[2]
    / "crates/vpr-owner-lab/src/rt1_visitor_conversation.rs"
)


class Rt1StartupCancellationGate(unittest.TestCase):
    def test_startup_does_not_call_unbounded_provider(self):
        source = SOURCE.read_text(encoding="utf-8")
        start = source.split("    pub fn start(", 1)[1].split("    /// # Errors", 1)[0]
        self.assertIn("RT1_PROVIDER_STARTUP_REQUIRES_BOUNDED_CANCELLATION", start)
        self.assertIn("manager.authorize_persona(", start)
        self.assertIn("manager.close(&id)", start)
        self.assertNotRegex(start, r"\.start_visitor\s*\(")
        self.assertNotIn("manager.provider_dispatch(", start)
        self.assertNotIn("std::thread::spawn(", start)
        # No dormant provider-dispatch API may bypass this fail-closed entry.
        session = (SOURCE.parent / "rt1_visitor_session.rs").read_text(encoding="utf-8")
        manager = (SOURCE.parent / "rt1_visitor_session_manager.rs").read_text(encoding="utf-8")
        workspace = (SOURCE.parent / "rt1_workspace_service.rs").read_text(encoding="utf-8")
        self.assertNotIn("fn provider_dispatch<", session)
        self.assertNotIn("fn provider_dispatch<", manager)
        self.assertNotIn("fn guarded_provider_dispatch<", workspace)
        self.assertNotIn("fn guarded_dispatch_under_transaction<", workspace)


if __name__ == "__main__":
    unittest.main()
