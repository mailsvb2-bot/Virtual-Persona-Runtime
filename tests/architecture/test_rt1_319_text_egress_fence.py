"""RT1 text must use the SAME canonical owner/visitor publication gate as
provider cancellation. These source assertions complement real Rust tests;
they do not prove a remote D-ID RTP STOP or a browser delivery fence.
"""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]
OWNER = ROOT / "crates/vpr-owner-lab/src"
BRIDGE = OWNER / "rt1_visitor_text_egress.rs"


class Rt1TextEgressFenceTests(unittest.TestCase):
    def test_visitor_text_uses_canonical_engine_under_revocable_admission(self):
        source = BRIDGE.read_text(encoding="utf-8")
        parent = (OWNER / "rt1_visitor_conversation.rs").read_text(encoding="utf-8")
        self.assertIn('mod text_egress;', parent)
        match = re.search(
            r"pub fn text_turn\(&self,.*?\) -> Result<LabTextResult, String> \{"
            r"(?P<body>.*?)\n    \}\n",
            source,
            re.S,
        )
        self.assertIsNotNone(match, "Visitor text entrypoint must exist")
        body = match.group("body")
        self.assertIn("self.remote.is_some()", body)
        self.assertIn("self.interrupt.backend_stop.lock().is_none()", body)
        self.assertIn("self.manager.begin_provider_egress(&self.id)?", body)
        self.assertRegex(body, r"self\s*\.engine\s*\.lock\(\)\s*\.text_turn\(text, move \|handle\|")
        self.assertIn("handle.interrupt()", body)
        self.assertIn("self.manager.authorize(&self.id)?", body)
        self.assertIn("self.interrupt.cancelled.load(Ordering::Acquire)", body)
        self.assertNotIn("compute_buffered(", body)
        self.assertNotIn("std::thread::spawn(", body)

    def test_owner_pause_and_visitor_egress_share_publication_transaction(self):
        workspace = (OWNER / "rt1_workspace_service.rs").read_text(encoding="utf-8")
        manager = (OWNER / "rt1_visitor_session_manager.rs").read_text(encoding="utf-8")
        session = (OWNER / "rt1_visitor_session.rs").read_text(encoding="utf-8")
        self.assertIn("self.change_publication_and_collect(", workspace)
        self.assertIn("let _transaction = self.transaction.lock();", workspace)
        self.assertIn("pub(crate) fn admit_visitor_provider_egress", workspace)
        admission = workspace.split("pub(crate) fn admit_visitor_provider_egress", 1)[1].split(
            "/// Invalidate all pre-crash", 1
        )[0]
        self.assertIn("let _transaction = self.transaction.lock();", admission)
        self.assertIn("workspace.authorize_visitor(", admission)
        self.assertIn("register()", admission)
        self.assertNotIn(".text_turn(", admission)
        self.assertNotIn(".voice_turn(", admission)
        self.assertIn(".admit_provider_egress(|| session.egress.admit(&session.revoked))", manager)
        self.assertRegex(session, r"self\.workspace\s*\.admit_visitor_provider_egress\(")

    def test_lease_tracks_inflight_until_completion_not_remote_stop(self):
        lease = (OWNER / "rt1_visitor_egress.rs").read_text(encoding="utf-8")
        self.assertIn("impl Drop for VisitorEgressPermit", lease)
        self.assertIn("revoked.load(Ordering::Acquire)", lease)
        self.assertIn("self.state.drained.notify_all()", lease)
        self.assertIn("NOT a remote audio/video RTP STOP receipt", lease)


if __name__ == "__main__":
    unittest.main()
