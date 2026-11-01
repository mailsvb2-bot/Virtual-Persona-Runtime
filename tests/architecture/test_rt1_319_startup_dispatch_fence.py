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
        self.assertIn("engine.start_visitor(OwnerLabStartRequest { consent })", canonical)
        self.assertIn("manager.open(visitor_token", canonical)
        self.assertIn("manager.authorize_persona(", canonical)
        self.assertGreaterEqual(len(re.findall(r"manager\s*\.\s*authorize_persona\s*\(", canonical)), 2)
        self.assertIn("interrupt.stop()", canonical)
        self.assertIn("engine.stop_visitor_provider_confirmed()", canonical)
        self.assertNotIn("RemoteOperationClient::new(", canonical)
        self.assertNotIn("std::thread::spawn(", canonical)
        self.assertLess(canonical.index("manager.open("), canonical.index("engine.start_visitor("))

        self.assertIn("RemoteOperationClient::new(", diagnostic)
        self.assertRegex(diagnostic, r"client\s*\.\s*create\s*\(\s*&operation_id\s*,\s*1\s*\)")
        self.assertRegex(diagnostic, r"client\s*\.\s*offer\s*\(\s*&operation_id\s*,\s*&session_id\s*\)")
        self.assertIn("stop_remote.revoke_confirmed()", diagnostic)
        self.assertLess(diagnostic.index("manager.open("),
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


    def test_http_registration_rechecks_revocation_and_drops_lock_before_cleanup(self):
        http = (SOURCE.parent / "http_rt1.rs").read_text(encoding="utf-8")
        start = http.split("fn visitor_start(", 1)[1].split("fn lookup(", 1)[0]
        registration = start.split("active.insert(session_id.clone(), Arc::new(conversation));", 1)[1]
        self.assertIn("session.is_active()", registration)
        self.assertIn("active.remove(&session_id)", registration)
        self.assertIn("drop(active);", registration)
        self.assertIn("session.close().is_err()", registration)
        self.assertIn('"RT1_REMOTE_MEDIA_STOP_UNCONFIRMED"', registration)
        self.assertLess(registration.index("active.remove(&session_id)"),
                        registration.index("session.close().is_err()"))
        self.assertLess(registration.index("session.close().is_err()"),
                        registration.index("Ok(json_response("))

    def test_http_start_failures_preserve_remote_stop_uncertainty(self):
        http = (SOURCE.parent / "http_rt1.rs").read_text(encoding="utf-8")
        start = http.split("fn visitor_start(", 1)[1].split("fn lookup(", 1)[0]
        for failure in (
            "RT1_VISITOR_REVOKED_DURING_START",
            "RT1_SESSION_ENTROPY_UNAVAILABLE",
            "RT1_SESSION_COLLISION",
        ):
            window = start.split('"' + failure + '"', 1)[0]
            self.assertIn("conversation.close().is_err()", window)
            self.assertIn('"RT1_REMOTE_STOP_UNCONFIRMED"', window)
            self.assertLess(window.rfind("conversation.close().is_err()"),
                            window.rfind('"RT1_REMOTE_STOP_UNCONFIRMED"'))

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
