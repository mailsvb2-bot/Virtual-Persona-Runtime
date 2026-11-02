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
        self.assertIn("engine.start_visitor_with_interrupt(", canonical)
        self.assertIn("*startup_interrupt.turn.lock() = Some(handle.clone());", canonical)
        self.assertIn("if startup_interrupt.cancelled.load(Ordering::Acquire)", canonical)
        self.assertIn("handle.interrupt()", canonical)
        self.assertLess(canonical.index("*startup_interrupt.turn.lock()"),
                        canonical.index("*interrupt.turn.lock() = None"))
        engine = (SOURCE.parent / "state.rs").read_text(encoding="utf-8")
        session_revocation = (SOURCE.parent / "state" / "session_revocation.rs").read_text(encoding="utf-8")
        self.assertIn("register_interrupt(turn.interrupt_handle());", engine)
        self.assertIn("self.start_scoped(request, LabSessionAudience::Visitor, register_interrupt)",
                      session_revocation)
        self.assertIn("manager.open_with_fast_revoke(", canonical)
        self.assertIn("fast_revoke.revoke_authority_only()", canonical)
        self.assertIn("move || on_revoke.stop()", canonical)
        self.assertIn("manager.authorize_persona(", canonical)
        self.assertGreaterEqual(len(re.findall(r"manager\s*\.\s*authorize_persona\s*\(", canonical)), 2)
        self.assertIn("interrupt.stop()", canonical)
        self.assertIn("engine.stop_visitor_provider_confirmed()", canonical)
        self.assertNotIn("RemoteOperationClient::new(", canonical)
        self.assertNotIn("std::thread::spawn(", canonical)
        self.assertLess(canonical.index("manager.open_with_fast_revoke("),
                        canonical.index("engine.start_visitor_with_interrupt("))

        self.assertIn("RemoteOperationClient::new(", diagnostic)
        self.assertRegex(diagnostic, r"client\s*\.\s*create\s*\(\s*&operation_id\s*,\s*1\s*\)")
        self.assertRegex(diagnostic, r"client\s*\.\s*offer\s*\(\s*&operation_id\s*,\s*&session_id\s*\)")
        self.assertIn("stop_remote.revoke_confirmed()", diagnostic)
        self.assertIn("fast_interrupt.revoke_authority_only()", diagnostic)
        self.assertLess(diagnostic.index("manager.open_with_fast_revoke("),
                        re.search(r"client\s*\.\s*create\s*\(", diagnostic).start())
        self.assertGreaterEqual(diagnostic.count("manager.authorize(&id)"), 2)
        # SDP must never be returned without rechecking the current persona
        # publication and remote STOP outcome after offer construction.
        self.assertRegex(diagnostic, r"manager\s*\.\s*authorize_persona\(&id, &persona_id, persona_version\)\s*\.is_err\(\)")
        self.assertIn("remote.is_revoked()", diagnostic)
        self.assertIn("interrupt.cancelled.load(Ordering::Acquire)", diagnostic)
        self.assertIn("return Err(revoked_diagnostic_offer_error(&manager, &id, &remote));", diagnostic)
        self.assertIn('"RT1_VISITOR_REVOKED_DURING_OFFER".into()', source)
        self.assertIn('"RT1_REMOTE_MEDIA_STOP_UNCONFIRMED".into()', source)
        self.assertLess(diagnostic.index("let bundle = started.map_err"),
                        diagnostic.rindex(".authorize_persona(&id, &persona_id, persona_version)"))


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
        self.assertIn("session.mark_revoked();", manager)
        self.assertLess(manager.index("session.mark_revoked();"),
                        manager.index("PendingVisitorRevocations { sessions }"))


    def test_http_registration_rechecks_revocation_and_drops_lock_before_cleanup(self):
        http = (SOURCE.parent / "http_rt1.rs").read_text(encoding="utf-8")
        start = http.split("fn visitor_start(", 1)[1].split("fn lookup(", 1)[0]
        helper = http.split("fn register_visitor<T>(", 1)[1].split("fn visitor_start(", 1)[0]
        self.assertIn("entry.insert(Arc::clone(conversation))", helper)
        self.assertIn("if !active_now(conversation.as_ref())", helper)
        self.assertIn("Arc::ptr_eq(current, conversation)", helper)
        self.assertIn("active.remove(session_id)", helper)
        self.assertLess(helper.index("entry.insert(Arc::clone(conversation))"),
                        helper.index("if !active_now(conversation.as_ref())"))
        self.assertIn("    }\n    if !active_now(", helper)
        registration = start.split("match register_visitor(", 1)[1]
        self.assertIn("Rt1VisitorConversation::is_active", registration)
        self.assertIn("conversation.close().is_err()", registration)
        self.assertIn('"RT1_REMOTE_MEDIA_STOP_UNCONFIRMED"', registration)
        self.assertLess(registration.index("conversation.close().is_err()"),
                        registration.index("Ok(json_response("))

    def test_http_start_preallocates_retry_id_and_quarantines_late_revoke(self):
        http = (SOURCE.parent / "http_rt1.rs").read_text(encoding="utf-8")
        start = http.split("fn visitor_start(", 1)[1].split("fn lookup(", 1)[0]
        # Entropy must be consumed BEFORE any external provider CREATE.
        self.assertLess(start.index("new_visitor_session_id()?"),
                        start.index("Rt1VisitorConversation::start("))
        self.assertNotIn("getrandom::fill(", start)
        # No pre-registration authority check may drop the only STOP target.
        self.assertLess(start.index("match register_visitor("),
                        start.index("let voice_enabled = conversation.supports_private_voice();"))
        self.assertLess(start.index("match register_visitor("),
                        start.index("if !conversation.is_active()"))
        late = start.split("if !conversation.is_active()", 1)[1]
        self.assertIn("finalize_registered_stop(", late)
        self.assertIn("conversation.close()", late)
        self.assertIn('"RT1_REMOTE_MEDIA_STOP_UNCONFIRMED"', late)
        self.assertIn('"RT1_VISITOR_REVOKED_DURING_START"', late)
        # A collision is still denied and attempts STOP, never returns SDP.
        collision = start.split("Err(RegistrationFailure::Collision)", 1)[1]
        self.assertIn("conversation.close().is_err()", collision)
        self.assertIn('"RT1_SESSION_COLLISION"', collision)

    def test_failed_diagnostic_start_cannot_hide_unconfirmed_remote_stop(self):
        source = SOURCE.read_text(encoding="utf-8")
        diagnostic = source.split("    pub fn start_diagnostic(", 1)[1].split(
            "    /// Whether this visitor", 1
        )[0]
        failure = diagnostic.split("let bundle = started.map_err(|error| {", 1)[1].split(
            "        })?;", 1
        )[0]
        self.assertIn("manager.close(&id)", failure)
        self.assertIn("remote.revoke_confirmed().is_err()", failure)
        self.assertIn('"RT1_REMOTE_MEDIA_STOP_UNCONFIRMED".to_owned()', failure)
        self.assertLess(failure.index("remote.revoke_confirmed().is_err()"),
                        failure.index("error\n"))


if __name__ == "__main__":
    unittest.main()
