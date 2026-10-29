//! Session authority operations are kept separate from the mutable conversation
//! engine so HTTP teardown can obtain a revoke-only handle without holding
//! the engine mutex during provider voice generation.
use super::{LabError, OwnerLabEngine};
use vpr_domain::RealtimeSessionState;
use vpr_runtime::{ActiveSession, SessionRevocationHandle};

impl OwnerLabEngine {
    /// Returns a revoke-only capability tied to this exact runtime session.
    /// It can invalidate provider permits without waiting for the Owner Lab engine mutex.
    #[must_use]
    pub fn session_revocation_handle(&self) -> Option<SessionRevocationHandle> {
        self.session.as_ref().map(ActiveSession::revocation_handle)
    }

    /// Stop-only capability for a server-owned LiveKit avatar publisher.
    /// Unlike a voice turn this cannot publish content, and does not require
    /// locking the conversation engine after admission.
    #[must_use]
    pub fn visitor_avatar_stop_handle(&self) -> Option<vpr_runtime::RealtimeAvatarStopHandle> {
        if self.session_audience != Some(super::LabSessionAudience::Visitor) {
            return None;
        }
        self.avatar
            .as_ref()?
            .stop_handle(std::sync::Arc::clone(&self.provider))
    }

    /// Withdraws runtime authority without waiting for an in-flight voice worker
    /// or an unreliable remote-provider close. Teardown must call this BEFORE
    /// waiting for quiescence: a timed-out stream must not leave the session active.
    ///
    /// # Errors
    /// Returns a stable state/runtime error; does not perform provider I/O.
    pub fn revoke_authority(&mut self) -> Result<(), LabError> {
        let session = self.session.as_mut().ok_or(LabError::InvalidState)?;
        match session.state() {
            RealtimeSessionState::Active => session.revoke().map_err(LabError::Runtime)?,
            RealtimeSessionState::Revoked => {}
            RealtimeSessionState::Created
            | RealtimeSessionState::Draining
            | RealtimeSessionState::Closed => return Err(LabError::InvalidState),
        }
        self.cancel_avatar_preparation();
        Ok(())
    }
    /// Stop the canonical visitor provider even when owner revocation has
    /// already withdrawn the local session permit. Do not confuse a revoked
    /// permit with a confirmed remote provider shutdown.
    ///
    /// # Errors
    /// Propagates a failed provider close; a caller must retain the session
    /// for cleanup retry rather than acknowledge STOP prematurely.
    pub fn stop_visitor_provider_confirmed(&mut self) -> Result<(), LabError> {
        if self.session_audience != Some(super::LabSessionAudience::Visitor) {
            return Err(LabError::InvalidState);
        }
        // The owner may have revoked this session via a stop-only capability
        // before this call. Closing a revoked provider is still mandatory.
        self.close_avatar_resource()?;
        self.close()
    }
}
