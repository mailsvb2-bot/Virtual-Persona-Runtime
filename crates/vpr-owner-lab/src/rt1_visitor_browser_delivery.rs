//! Final local RT1 HTTP buffered response lease. No second runtime or sender.
use super::{Ordering, Rt1VisitorConversation};

/// Keeps the canonical visitor provider-egress lease alive through the HTTP
/// response write. Dropping it signals LOCAL delivery quiescence only.
pub struct Rt1BrowserDeliveryPermit {
    _lease: crate::rt1_visitor_session_manager::VisitorEgressPermit,
}

impl Rt1VisitorConversation {
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
