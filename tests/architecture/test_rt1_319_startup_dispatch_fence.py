"""RT1 startup registers bounded remote cancellation before admitting media."""
from pathlib import Path
import re
import unittest

SOURCE = (
    Path(__file__).resolve().parents[2]
    / "crates/vpr-owner-lab/src/rt1_visitor_conversation.rs"
)


class Rt1StartupCancellationGate(unittest.TestCase):
    def test_canonical_provider_start_is_fenced_and_diagnostic_is_explicit(self):
        source = SOURCE.read_text(encoding="utf-8")
        canonical = source.split("    pub fn start(", 1)[1].split(
            "    pub fn start_diagnostic(", 1
        )[0]
        diagnostic = source.split("    pub fn start_diagnostic(", 1)[1].split(
            "    /// Whether this visitor", 1
        )[0]
        self.assertIn("engine.start_visitor(OwnerLabStartRequest { consent })", canonical)
        self.assertIn("manager.open(visitor_token", canonical)
        self.assertIn("manager.authorize_persona(", canonical)
        self.assertGreaterEqual(len(re.findall(r"manager\s*\.\s*authorize_persona\s*\(", canonical)), 2)
        self.assertIn("interrupt.stop()", canonical)
        self.assertIn("engine.stop_visitor_provider_confirmed()", canonical)
        self.assertNotIn("RemoteOperationClient::new(", canonical)
        self.assertNotIn("std::thread::spawn(", canonical)
        self.assertLess(canonical.index("manager.open("), canonical.index("engine.start_visitor("))

        self.assertIn("RemoteOperationClient::new(", diagnostic)
        self.assertRegex(diagnostic, r"client\s*\.\s*create\s*\(\s*&operation_id\s*,\s*1\s*\)")
        self.assertRegex(diagnostic, r"client\s*\.\s*offer\s*\(\s*&operation_id\s*,\s*&session_id\s*\)")
        self.assertIn("stop_remote.revoke_confirmed()", diagnostic)
        self.assertLess(diagnostic.index("manager.open("),
                        re.search(r"client\s*\.\s*create\s*\(", diagnostic).start())
        self.assertGreaterEqual(diagnostic.count("manager.authorize(&id)"), 2)

        http = (SOURCE.parent / "http_rt1.rs").read_text(encoding="utf-8")
        self.assertIn('std::env::var("VPR_RT1_DIAGNOSTIC_RTP")', http)
        self.assertIn("Rt1VisitorConversation::start_diagnostic(", http)
        self.assertIn("Rt1VisitorConversation::start(", http)
        self.assertIn("Rt1VisitorConversation::start(manager", http)

        session = (SOURCE.parent / "rt1_visitor_session.rs").read_text(encoding="utf-8")
        manager = (SOURCE.parent / "rt1_visitor_session_manager.rs").read_text(encoding="utf-8")
        workspace = (SOURCE.parent / "rt1_workspace_service.rs").read_text(encoding="utf-8")
        self.assertNotIn("fn provider_dispatch<", session)
        self.assertNotIn("fn provider_dispatch<", manager)
        self.assertNotIn("fn guarded_provider_dispatch<", workspace)
        self.assertNotIn("fn guarded_dispatch_under_transaction<", workspace)


if __name__ == "__main__":
    unittest.main()
