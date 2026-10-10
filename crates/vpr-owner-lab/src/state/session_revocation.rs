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
}
