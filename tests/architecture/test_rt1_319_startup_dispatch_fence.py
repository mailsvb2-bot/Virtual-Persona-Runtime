"""Guard RT1 against unbounded provider startup under owner transaction."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
CRATE = ROOT / "crates/vpr-owner-lab/src"


class Rt1StartupFenceTest(unittest.TestCase):
    def test_unbounded_visitor_startup_fails_closed_before_provider(self):
        conversation = (CRATE / "rt1_visitor_conversation.rs").read_text(encoding="utf8")
        start = conversation.split("    pub fn start(", 1)[1].split(
            "    /// # Errors\n    /// Denies revoked visitor authority", 1
        )[0]
        self.assertNotIn(".start_visitor(", start)
        self.assertNotIn("manager.provider_dispatch(", start)
        self.assertIn("RT1_PROVIDER_STARTUP_REQUIRES_BOUNDED_CANCEL_AND_STOP", start)
        self.assertLess(start.index("manager.close(&id)"), start.index("Err(\"RT1_PROVIDER_STARTUP_REQUIRES_BOUNDED_CANCEL_AND_STOP\""))


if __name__ == "__main__":
    unittest.main()
