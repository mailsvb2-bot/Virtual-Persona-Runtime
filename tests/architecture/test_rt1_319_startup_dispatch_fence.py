"""Guard against reintroducing unfenced visitor provider startup (#319)."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
CRATE = ROOT / "crates/vpr-owner-lab/src"


class Rt1StartupFenceTest(unittest.TestCase):
    def test_startup_dispatch_is_revoke_serialized(self):
        conversation = (CRATE / "rt1_visitor_conversation.rs").read_text(encoding="utf8")
        workspace = (CRATE / "rt1_workspace_service.rs").read_text(encoding="utf8")
        manager = (CRATE / "rt1_visitor_session_manager.rs").read_text(encoding="utf8")
        self.assertIn("manager.provider_dispatch(&id, || {", conversation)
        self.assertRegex(
            conversation,
            r"\bengine\s*\.\s*start_visitor\s*\(\s*OwnerLabStartRequest\s*\{\s*consent\s*\}\s*\)",
        )
        self.assertIn("fn guarded_provider_dispatch<T>", workspace)
        fenced = workspace.split("fn guarded_provider_dispatch<T>", 1)[1].split(
            "    /// Serialize one short, bounded local enqueue", 1
        )[0]
        self.assertIn("guarded_dispatch_under_transaction(", fenced)
        self.assertIn("self.transaction,", fenced)
        self.assertIn("workspace.authorize_visitor(", fenced)
        helper = workspace.split("fn guarded_dispatch_under_transaction<T>", 1)[1].split(
            "/// Validate the current publication before inserting", 1
        )[0]
        self.assertIn("transaction.lock()", helper)
        self.assertIn("authorize()?", helper)
        self.assertIn("dispatch()", helper)
        self.assertLess(helper.index("transaction.lock()"), helper.index("authorize()?"))
        self.assertLess(helper.index("authorize()?"), helper.index("dispatch()"))
        self.assertIn("session.authority.provider_dispatch(", manager)


if __name__ == "__main__":
    unittest.main()
