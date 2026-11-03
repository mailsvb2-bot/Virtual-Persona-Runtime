"""Regression: RT1 visitor text must not dispatch unfenced LLM calls.

This is a temporary fail-closed contract, NOT closure of issue #319.
"""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]
BRIDGE = ROOT / "crates/vpr-owner-lab/src/rt1_visitor_conversation.rs"


class Rt1TextEgressFenceTests(unittest.TestCase):
    def test_visitor_text_provider_egress_remains_closed(self):
        source = BRIDGE.read_text(encoding="utf-8")
        match = re.search(
            r"pub fn text_turn\(&self,.*?\) -> Result<LabTextResult, String> \{"
            r"(?P<body>.*?)\n    \}\n",
            source,
            re.S,
        )
        self.assertIsNotNone(match, "Visitor text gate must be present")
        body = match.group("body")
        self.assertIn("self.manager.authorize(&self.id)?", body)
        self.assertIn("pending revocation fence (#319)", body)
        self.assertNotIn("engine.text_turn(", body)
        self.assertNotIn("compute_buffered(", body)


if __name__ == "__main__":
    unittest.main()
