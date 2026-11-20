"""Port RT1 #319 startup safety assertions from the historical remote branch.

The original `fix/rt1-319-serialized-start-egress-20261025` test assumed
startup lived in a single obsolete module. These checks target today's one
canonical split path. They complement (not replace) runtime Rust/Playwright tests.
No provider calls or new sender are introduced.
"""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
OWNER = ROOT / "crates/vpr-owner-lab/src"


def source(name: str) -> str:
    return (OWNER / name).read_text(encoding="utf-8")


class Rt1StartupDispatchFenceTests(unittest.TestCase):
    def test_http_admits_and_reserves_before_provider_allocation(self):
        http = source("http_rt1_start.rs")
        start = http.split("pub(super) fn visitor_start(", 1)[1]
        self.assertLess(start.index("if !body.consent"), start.index("admitted_visitor_manager"))
        self.assertLess(start.index("admitted_visitor_manager(&body.visitor_token)?"),
                        start.index("reserve_session_identity(state)?"))
        self.assertLess(start.index("reserve_session_identity(state)?"),
                        start.index("selected_provider_bundle(&session_id)?"))
        self.assertLess(start.index("selected_provider_bundle(&session_id)?"),
                        start.index("engine_bootstrap::assemble(providers, true)"))
        self.assertIn("register_visitor(", start)
        self.assertLess(start.index("register_visitor("), start.index("conversation.supports_private_voice()"))
        self.assertIn("finalize_registered_stop(", start)
        self.assertIn("RT1_REMOTE_MEDIA_STOP_UNCONFIRMED", start)

    def test_canonical_visitor_registers_revoke_before_engine_start(self):
        conversation = source("rt1_visitor_conversation.rs")
        start = conversation.split("fn start_inner(", 1)[1].split(
            "    pub fn start_diagnostic(", 1
        )[0]
        self.assertLess(start.index("manager.open_with_fast_revoke("),
                        start.index("manager.authorize_persona("))
        self.assertLess(start.index("manager.authorize_persona("),
                        start.index("helpers::start_engine_with_provider_egress("))
        self.assertLess(start.index("register(remote)"),
                        start.index("helpers::start_engine_with_provider_egress("))
        self.assertIn("remote.request_revoke()", start)
        self.assertIn("manager.close(&id)", start)
        self.assertIn("engine.stop_visitor_provider_confirmed()", start)
        self.assertIn("interrupt.revoke_authority_only()", start)
        self.assertIn("RT1_PROVIDER_START_REVOKED_STOP_UNCONFIRMED", start)

        helper = source("rt1_visitor_conversation_helpers.rs")
        dispatch = helper.split("pub(super) fn start_engine_with_provider_egress(", 1)[1].split(
            "pub(super) fn cleanup_failed_start(", 1
        )[0]
        self.assertLess(dispatch.index("manager.begin_provider_egress(session_id)?"),
                        dispatch.index("engine.start_visitor_with_interrupt("))
        self.assertLess(dispatch.index("*startup_interrupt.turn.lock() = Some(handle.clone())"),
                        dispatch.index("let _ = handle.interrupt()"))
        self.assertIn("startup_interrupt", dispatch)
        self.assertNotIn("std::thread::spawn(", dispatch)

    def test_diagnostic_registers_retryable_stop_before_external_create(self):
        conversation = source("rt1_visitor_conversation.rs")
        diagnostic = conversation.split("pub fn start_diagnostic(", 1)[1].split(
            "pub fn ", 1
        )[0]
        self.assertLess(diagnostic.index("manager.open_with_fast_revoke("),
                        diagnostic.index("register_cleanup(Arc::clone(&remote))"))
        self.assertLess(diagnostic.index("register_cleanup(Arc::clone(&remote))"),
                        diagnostic.index(".create(&operation_id, 1)"))
        self.assertLess(diagnostic.index(".create(&operation_id, 1)"),
                        diagnostic.index("remote.record_created_session(&session_id)?"))
        self.assertLess(diagnostic.index("remote.record_created_session(&session_id)?"),
                        diagnostic.index(".offer(&operation_id, &session_id)"))
        # No SDP may escape after owner correction/revocation, and a failed
        # STOP keeps an explicit unconfirmed state instead of a success claim.
        self.assertLess(diagnostic.index(".offer(&operation_id, &session_id)"),
                        diagnostic.rindex(".authorize_persona(&id, &persona_id, persona_version)"))
        self.assertIn("revoked_diagnostic_offer_error(&manager, &id, &remote)", diagnostic)
        self.assertIn("RT1_REMOTE_MEDIA_STOP_UNCONFIRMED", diagnostic)
        self.assertNotIn("std::thread::spawn(", diagnostic)

    def test_start_uses_same_cancellable_browser_response_fence(self):
        http = source("http_rt1.rs")
        transport = source("http_rt1_transport.rs")
        self.assertIn("guarded_reply(visitor_start(request, state))", http)
        self.assertIn("conversation.begin_browser_tcp_delivery(writer)", transport)
        self.assertIn("conversation.revoke_undelivered_start()", transport)


if __name__ == "__main__":
    unittest.main()
