//! Canonical RT1 visitor SDP/ICE egress through the existing publication lease.
//! Provider turn cancellation reuses the single OwnerLabEngine.
use super::{Arc, Ordering, Rt1VisitorConversation};
use crate::state::OwnerLabTurnInput;

impl Rt1VisitorConversation {
    pub fn submit_answer(&self, sdp: &str) -> Result<(), String> {
        // Registration linearizes with the durable owner publication transaction.
        // Provider I/O starts only after the transaction releases its lock.
        let _egress = self.manager.begin_provider_egress(&self.id)?;
        if self.interrupt.cancelled.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        let result = if let Some(remote) = &self.remote {
            // The durable worker serializes SDP with its authenticated STOP gate.
            remote.submit_answer(sdp)
        } else {
            self.apply_visitor_signaling(OwnerLabTurnInput::Answer(
                vpr_integration::WebRtcSessionDescription {
                    kind: "answer".into(),
                    sdp: sdp.to_owned(),
                },
            ))
        };
        // Even a provider error cannot bypass revoke and retain stale success.
        self.manager.authorize(&self.id)?;
        if self.interrupt.cancelled.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        result
    }

    /// Forward ICE exclusively through a cancellation-registered canonical
    /// provider turn. Diagnostic worker sessions deny separate ICE egress.
    ///
    /// # Errors
    /// Rejects stale authority, diagnostic ICE and provider failures.
    pub fn submit_ice(&self, candidate: vpr_integration::WebRtcIceCandidate) -> Result<(), String> {
        if self.remote.is_some() {
            return Err("RT1_VISITOR_ICE_DENIED".into());
        }
        let _egress = self.manager.begin_provider_egress(&self.id)?;
        if self.interrupt.cancelled.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        let result = self.apply_visitor_signaling(OwnerLabTurnInput::Ice(candidate));
        self.manager.authorize(&self.id)?;
        if self.interrupt.cancelled.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        result
    }

    /// Register the canonical turn before provider signaling, without a second
    /// engine or independent authority. Revoke can interrupt this turn while
    /// another thread holds the conversation engine mutex.
    fn apply_visitor_signaling(&self, input: OwnerLabTurnInput) -> Result<(), String> {
        let callback = Arc::clone(&self.interrupt);
        let result = self
            .engine
            .lock()
            .apply_with_interrupt(input, move |handle| {
                *callback.turn.lock() = Some(handle.clone());
                if callback.cancelled.load(Ordering::Acquire) {
                    let _ = handle.interrupt();
                }
            })
            .map_err(|error| error.code().to_owned());
        *self.interrupt.turn.lock() = None;
        result
    }
}
