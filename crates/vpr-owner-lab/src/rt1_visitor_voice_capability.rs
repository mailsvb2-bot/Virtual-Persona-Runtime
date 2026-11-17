//! Canonical RT1 voice exposure: one reviewed visitor permit and the existing
//! operation-owned worker with independently verified audio/video shutdown.
use super::{Ordering, Rt1VisitorConversation};

fn verified_worker_voice_egress_active(
    verified_worker_voice: bool,
    remote_worker_active: bool,
    cancelled: bool,
) -> bool {
    verified_worker_voice && remote_worker_active && !cancelled
}

impl Rt1VisitorConversation {
    /// Advertise microphone speech only when this session was admitted
    /// through the verified worker, not through diagnostic RTP or D-ID.
    /// The durable worker STOP remains distinct from the engine mutex.
    #[must_use]
    pub fn supports_private_voice(&self) -> bool {
        let remote_worker_active = self
            .remote
            .as_ref()
            .is_some_and(|worker| !worker.is_revoked());
        verified_worker_voice_egress_active(
            self.verified_worker_voice,
            remote_worker_active,
            self.interrupt.cancelled.load(Ordering::Acquire),
        ) && self.manager.authorize(&self.id).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::verified_worker_voice_egress_active;

    #[test]
    fn only_verified_active_worker_with_live_authority_can_dispatch_voice_or_text() {
        assert!(verified_worker_voice_egress_active(true, true, false));
        assert!(!verified_worker_voice_egress_active(false, true, false));
        assert!(!verified_worker_voice_egress_active(true, false, false));
        assert!(!verified_worker_voice_egress_active(true, true, true));
    }
}
