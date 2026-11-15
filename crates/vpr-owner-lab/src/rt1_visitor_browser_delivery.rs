//! Final local RT1 HTTP buffered response lease. No second runtime or sender.
use super::{Ordering, Rt1VisitorConversation};
use vpr_runtime::{SessionRevocationHandle, TcpResponseRegistration, TcpWriteRevocation};

/// Keeps the canonical visitor provider-egress lease alive through the HTTP
/// response write. Dropping it signals LOCAL delivery quiescence only.
pub struct Rt1BrowserDeliveryPermit {
    _lease: crate::rt1_visitor_session_manager::VisitorEgressPermit,
}

impl Rt1VisitorConversation {
    /// START can allocate a sender, and SDP ANSWER/ICE may activate it before
    /// the final browser acknowledgement. If that response fails, revoke
    /// synchronously through the existing STOP reconciler. Local disconnect
    /// is never evidence of verified remote audio/video shutdown.
    pub fn revoke_undelivered_start(&self) {
        self.interrupt.revoke_authority_only();
        if let Some(remote) = &self.remote {
            remote.request_revoke();
        }
        let _ = self.manager.close(&self.id);
    }

    /// Acquire a final browser-delivery lease using the SAME publication
    /// admission gate as the canonical STT/LLM/TTS provider turns. The lease
    /// must live until the HTTP write finishes, not merely until JSON is built.
    ///
    /// A concurrent owner pause invalidates authority immediately; it must
    /// never claim confirmed sender shutdown while a delivery lease is held.
    /// This is a LOCAL quiescence fence, NOT remote audio/video STOP evidence.
    /// A stalled browser write may still outlive revocation; #319 requires a
    /// bounded cancellable network writer before this is a release gate.
    ///
    /// # Errors
    /// Denies revoked publication, concurrent provider turns, or stale permits.
    pub fn begin_browser_delivery(&self) -> Result<Rt1BrowserDeliveryPermit, String> {
        if self.interrupt.cancelled.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        let lease = self.manager.begin_provider_egress(&self.id)?;
        if !self.is_active() {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        Ok(Rt1BrowserDeliveryPermit { _lease: lease })
    }
}

/// Holds the visitor publication permit and the real browser TCP registration
/// until the outer HTTP proxy finishes writing the response.
pub struct Rt1BrowserDeliveryGuard {
    _permit: Rt1BrowserDeliveryPermit,
    _registration: TcpResponseRegistration,
}

impl Rt1VisitorConversation {
    /// Register the actual client socket against this visitor's canonical
    /// runtime session. Owner revoke shuts the socket through the same
    /// execution gate used by provider egress.
    ///
    /// # Errors
    /// Rejects a missing runtime session or revoked publication. A rejected
    /// writer is always aborted locally.
    pub fn begin_browser_tcp_delivery(
        &self,
        writer: TcpWriteRevocation,
    ) -> Result<Rt1BrowserDeliveryGuard, String> {
        let abort = writer.clone();
        let permit = match self.begin_browser_delivery() {
            Ok(permit) => permit,
            Err(error) => {
                abort.revoke();
                return Err(error);
            }
        };
        let session: Option<SessionRevocationHandle> = self.interrupt.session.lock().clone();
        let Some(session) = session else {
            abort.revoke();
            return Err("RT1_VISITOR_RUNTIME_SESSION_UNAVAILABLE".into());
        };
        let registration = session.register_tcp_response_writer(writer).map_err(|_| {
            abort.revoke();
            "RT1_VISITOR_DELIVERY_DENIED".to_owned()
        })?;
        if !self.is_active() {
            abort.revoke();
            drop(registration);
            return Err("RT1_VISITOR_REVOKED".into());
        }
        Ok(Rt1BrowserDeliveryGuard {
            _permit: permit,
            _registration: registration,
        })
    }
}
