"""RT1 #319: provider SDP/ICE must use publication admission and cancellable runtime."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
OWNER = ROOT / "crates/vpr-owner-lab/src"
BRIDGE = (OWNER / "rt1_visitor_conversation.rs").read_text(encoding="utf-8")
ENGINE = (OWNER / "state.rs").read_text(encoding="utf-8")
REMOTE = (OWNER / "rt1_remote_media_revocation.rs").read_text(encoding="utf-8")


def method_body(source: str, name: str) -> str:
    start = source.index("    " + ("pub " if name != "apply_visitor_signaling" else "") + "fn " + name + "(")
    body_start = source.index("{", start)
    depth = 0
    for position in range(body_start, len(source)):
        if source[position] == "{":
            depth += 1
        elif source[position] == "}":
            depth -= 1
            if not depth:
                return source[body_start + 1:position]
    raise AssertionError("unterminated " + name)


class SignalingProviderEgressFence(unittest.TestCase):
    def test_sdp_answer_admission_precedes_signaling_and_outlives_final_check(self):
        body = method_body(BRIDGE, "submit_answer")
        self.assertIn("self.manager.begin_provider_egress(&self.id)?", body)
        self.assertLess(body.index("begin_provider_egress"), body.index("remote.submit_answer(sdp)"))
        self.assertLess(body.index("begin_provider_egress"), body.index("self.apply_visitor_signaling("))
        self.assertLess(body.index("remote.submit_answer(sdp)"), body.index("self.manager.authorize(&self.id)?"))
        self.assertIn("self.interrupt.cancelled.load(Ordering::Acquire)", body)
        self.assertNotIn("compute_buffered(", body)

    def test_ice_admission_and_cancellation_before_remote_io(self):
        body = method_body(BRIDGE, "submit_ice")
        self.assertIn("if self.remote.is_some()", body)
        self.assertLess(body.index("if self.remote.is_some()"), body.index("begin_provider_egress"))
        self.assertLess(body.index("begin_provider_egress"), body.index("self.apply_visitor_signaling("))
        self.assertLess(body.index("self.apply_visitor_signaling("), body.index("self.manager.authorize(&self.id)?"))
        self.assertNotIn("compute_buffered(", body)

    def test_one_canonical_turn_registers_interrupt_before_dispatch(self):
        bridge = method_body(BRIDGE, "apply_visitor_signaling")
        self.assertIn(".apply_with_interrupt(input, move |handle|", bridge)
        self.assertIn("*callback.turn.lock() = Some(handle.clone())", bridge)
        self.assertIn("handle.interrupt()", bridge)
        self.assertIn("*self.interrupt.turn.lock() = None", bridge)
        runtime = method_body(ENGINE, "apply_with_interrupt")
        self.assertLess(runtime.index("register_interrupt(turn.interrupt_handle());"),
                        runtime.index("turn.submit_realtime_avatar_answer("))
        self.assertLess(runtime.index("register_interrupt(turn.interrupt_handle());"),
                        runtime.index("turn.submit_realtime_avatar_ice("))
        standard = method_body(ENGINE, "apply")
        self.assertIn("self.apply_with_interrupt(input, |_| {})", standard)
        self.assertNotIn("self.engine.lock()", bridge.split(".apply_with_interrupt", 1)[1])

    def test_durable_worker_sdp_and_stop_are_serialized(self):
        remote_answer = method_body(REMOTE, "submit_answer")
        remote_stop = method_body(REMOTE, "revoke_confirmed")
        self.assertIn("self.stop_gate.lock()", remote_answer)
        self.assertIn("self.stop_gate.lock()", remote_stop)
        self.assertIn("self.cancel_locked()?", remote_answer)


if __name__ == "__main__":
    unittest.main()
