//! Canonical RT1 voice exposure: one reviewed visitor permit and the existing
//! operation-owned worker with independently verified audio/video shutdown.
use super::{Ordering, Rt1VisitorConversation};

impl Rt1VisitorConversation {
    /// Advertise microphone speech only when this session was admitted
    /// through the verified worker, not through diagnostic RTP or D-ID.
    /// The durable worker STOP remains distinct from the engine mutex.
    #[must_use]
    pub fn supports_private_voice(&self) -> bool {
        self.verified_worker_voice
            && self
                .remote
                .as_ref()
                .is_some_and(|worker| !worker.is_revoked())
            && !self.interrupt.cancelled.load(Ordering::Acquire)
            && self.manager.authorize(&self.id).is_ok()
    }
}
