"""RT1 visitor microphone may use only canonical private Echo with independent STOP.

Guard the narrow, stop-capable buffered path while denying arbitrary sender
or client callback egress. Source checks are not live provider evidence.
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

    def test_buffered_voice_requires_stop_only_capability_and_canonical_runtime(self):
        body = method_body(self.source, "voice_turn")
        self.assertIn("self.remote.is_some()", body)
        self.assertIn("self.interrupt.cancelled.load(Ordering::Acquire)", body)
        self.assertIn("self.interrupt.backend_stop.lock().is_none()", body)
        self.assertIn("return unsupported_visitor_voice_egress()", body)
        self.assertIn("self.stage_microphone_audio(&sender, pcm_s16le_mono_16khz)?", body)
        self.assertIn("std::sync::mpsc::sync_channel(1)", body)
        self.assertGreaterEqual(body.count("self.manager.authorize(&self.id)?"), 2)
        self.assertIn(".voice_turn(pcm_s16le_mono_16khz, move |handle|", body)
        self.assertIn("*callback.turn.lock() = Some(handle.clone())", body)
        self.assertIn("handle.interrupt()", body)
        self.assertNotIn("emit_segment(", body)
        self.assertNotIn("std::thread::spawn(", body)

    def test_owner_revocation_dispatches_only_stop_capability(self):
        source = self.source
        self.assertIn("engine.visitor_avatar_stop_handle()", source)
        self.assertIn("self.backend_stop.lock().take()", source)
        self.assertIn("stop.stop()", source)
        self.assertIn("self.cancelled.store(true, Ordering::Release)", source)

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
