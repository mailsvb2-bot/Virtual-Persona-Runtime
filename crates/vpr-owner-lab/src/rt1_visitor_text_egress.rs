//! Canonical RT1 visitor LLM dispatch under the shared publication gate.
//! No separate LLM runtime, provider adapter or owner identity is created.
use super::{Arc, Ordering, Rt1VisitorConversation};
use crate::state::LabTextResult;

impl Rt1VisitorConversation {
    /// # Errors
    /// Denies revoked visitor authority or returns the canonical turn error.
    pub fn text_turn(&self, text: &str) -> Result<LabTextResult, String> {
        // The verified worker's Rt1RemoteMediaRevocation is the independent
        // dual-track STOP owner. It intentionally has no LiveKit-only backend
        // stop handle, so text and voice must share the same worker capability
        // predicate instead of rejecting every remote worker session.
        if !self.supports_private_voice() {
            return Err("RT1_VISITOR_TEXT_EGRESS_NOT_AUTHORIZED".into());
        }
        // This registration is serialized with the owner's durable
        // publication transaction. Network I/O runs AFTER the transaction
        // releases its lock, through the existing interruptible RT0 engine.
        let _egress = self.manager.begin_provider_egress(&self.id)?;
        if self.interrupt.cancelled.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        let callback = Arc::clone(&self.interrupt);
        let result = self
            .engine
            .lock()
            .text_turn(text, move |handle| {
                *callback.turn.lock() = Some(handle.clone());
                // A pause between publication admission and runtime turn
                // registration must cancel the new turn before provider dispatch.
                if callback.cancelled.load(Ordering::Acquire) {
                    let _ = handle.interrupt();
                }
            })
            .map_err(|error| error.code().to_owned());
        *self.interrupt.turn.lock() = None;
        // No buffered LLM result may escape after owner pause/unpublish.
        // The HTTP sender must also revalidate at its final delivery fence.
        self.manager.authorize(&self.id)?;
        if self.interrupt.cancelled.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        result
    }
}
