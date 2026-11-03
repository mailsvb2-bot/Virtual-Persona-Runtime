"""RT1 visitor microphone must remain fail-closed until #319 media egress is fenced.

The Rust unit tests for the denial helper alone do not prove that the public
voice entrypoints actually invoke it. Check the entrypoint bodies, and run
this guard in required CI. This is a source-level guard, not RT0 evidence.
"""

from pathlib import Path
import re
import unittest

SOURCE = (
    Path(__file__).resolve().parents[2]
    / "crates/vpr-owner-lab/src/rt1_visitor_conversation.rs"
)


def method_body(source: str, method: str) -> str:
    match = re.search(r"(?m)^\s*pub fn " + re.escape(method) + r"\s*\(", source)
    if match is None:
        raise AssertionError(f"Missing public RT1 visitor entrypoint: {method}")
    # A signature can span multiple lines. Find the opening body brace only
    # after the return type; there are no block braces in these signatures.
    start = source.find("{", match.end())
    if start < 0:
        raise AssertionError(f"Missing method body: {method}")
    depth = 0
    for index in range(start, len(source)):
        if source[index] == "{":
            depth += 1
        elif source[index] == "}":
            depth -= 1
            if depth == 0:
                return source[start + 1 : index].strip()
    raise AssertionError(f"Unbalanced method body: {method}")


class VisitorVoiceEgressGate(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = SOURCE.read_text(encoding="utf-8")

    def test_buffered_voice_entrypoint_is_denial_only(self):
        self.assertEqual(
            method_body(self.source, "voice_turn"),
            "unsupported_visitor_voice_egress()",
        )

    def test_streaming_voice_entrypoint_cannot_invoke_callback_or_provider(self):
        self.assertEqual(
            method_body(self.source, "voice_turn_streaming"),
            'Err("RT1_VOICE_PROVIDER_EGRESS_NOT_AUTHORIZED".into())',
        )

    def test_denial_helper_is_error_only(self):
        self.assertRegex(
            self.source,
            r'fn unsupported_visitor_voice_egress<T>\(\) -> Result<T, String> \{\s*'
            r'Err\("RT1_VOICE_PROVIDER_EGRESS_NOT_AUTHORIZED"\.into\(\)\)\s*\}',
        )


if __name__ == "__main__":
    unittest.main()
