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

    def test_local_rt1_http_and_browser_reuse_canonical_text_turn(self):
        http = (OWNER / "http_rt1.rs").read_text(encoding="utf-8")
        browser = (ROOT / "crates/vpr-owner-lab/ui/rt1.js").read_text(encoding="utf-8")
        html = (ROOT / "crates/vpr-owner-lab/ui/rt1.html").read_text(encoding="utf-8")
        self.assertIn('if path == "/api/rt1/visitor/text"', http)
        self.assertIn("guarded_reply(visitor_text(request, state))", http)
        match = re.search(
            r"fn visitor_text\(.*?\) -> Result<.*?\{(?P<body>.*?)\n\}",
            http, re.S,
        )
        self.assertIsNotNone(match)
        body = match.group("body")
        self.assertIn("let conversation = lookup(state, &body.session_id)?;", body)
        self.assertIn(".text_turn(&body.text)", body)
        self.assertIn("if !conversation.is_active()", body)
        self.assertIn('"RT1_VISITOR_TEXT_DENIED"', body)
        self.assertNotIn("state.engine.lock()", body)
        self.assertIn('api("/api/rt1/visitor/text", { session_id: id, text })', browser)
        self.assertIn('if (sessionId !== id || stopping) return;', browser)
        self.assertIn('id="visitor-text"', html)
        self.assertIn('id="send-text"', html)

    def test_buffered_text_and_voice_http_write_retains_final_canonical_lease(self):
        main = (OWNER / "main.rs").read_text(encoding="utf-8")
        http = (OWNER / "http_rt1.rs").read_text(encoding="utf-8")
        conversation = (OWNER / "rt1_visitor_conversation.rs").read_text(encoding="utf-8")
        self.assertIn("guarded_reply(visitor_text(request, state))", http)
        self.assertIn("guarded_reply(visitor_voice(request, state))", http)
        delivery = (OWNER / "rt1_visitor_browser_delivery.rs").read_text(encoding="utf-8")
        self.assertIn("mod browser_delivery;", conversation)
        self.assertIn("pub fn begin_browser_delivery(&self)", delivery)
        self.assertIn("self.manager.begin_provider_egress(&self.id)?", delivery)
        self.assertIn("if !self.is_active()", delivery)
        self.assertIn('http_rt1::respond_post(request, state)', main)
        self.assertIn("conversation.begin_browser_delivery()", http)
        self.assertIn("Ok(_lease)", http)
        self.assertIn("request.respond(routed.response)", http)
        self.assertIn("valid_post_headers(&request, &state.csrf_token, state.port)", http)
        self.assertIn('"RT1_VISITOR_DELIVERY_DENIED"', http)
        self.assertNotIn("state.engine.lock()", http.split("fn visitor_text(", 1)[1].split("fn visitor_voice(", 1)[0])

    def test_startup_sdp_uses_same_final_browser_egress_fence(self):
        # A successful start carries a live SDP offer and ICE credentials.
        # It must not bypass the text/voice browser delivery lease after a
        # concurrent owner pause. This is source coverage, NOT a live RTP test.
        http = (OWNER / "http_rt1.rs").read_text(encoding="utf-8")
        dispatch = http.split("pub(super) fn route_post(", 1)[1].split(
            "type Rt1HttpResult =", 1
        )[0]
        self.assertIn('if path == "/api/rt1/visitor/start"', dispatch)
        self.assertIn("guarded_reply(visitor_start(request, state))", dispatch)
        self.assertNotIn(
            '"/api/rt1/visitor/start" => visitor_start(request, state)',
            dispatch,
        )
        start = http.split("fn visitor_start(", 1)[1].split("fn lookup(", 1)[0]
        self.assertIn(
            "Result<(HttpResponse, Arc<Rt1VisitorConversation>), HttpResponse>",
            start,
        )
        self.assertIn("conversation,", start)
        writer = http.split("pub(super) fn respond_post(", 1)[1].split(
            "pub(super) fn route_post(", 1
        )[0]
        self.assertIn("conversation.begin_browser_delivery()", writer)
        self.assertIn("request.respond(routed.response)", writer)

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
