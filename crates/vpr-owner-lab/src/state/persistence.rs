use vpr_domain::{ClaimId, ClaimKind, RealtimeSessionState, Rt0ReasonCode};

use crate::owner_context::{ReviewedOwnerContext, ReviewedOwnerContextSnapshot};

use super::{LabError, OwnerLabEngine, map_owner_context_error};

impl OwnerLabEngine {
    /// Corrects one reviewed owner claim and commits the resulting snapshot atomically with
    /// durable persistence. The domain layer rolls back only the touched claim revision and
    /// PersonaVersion if persistence fails; readiness is reset only after a successful commit.
    ///
    /// # Errors
    /// Returns the canonical correction error, or `PersistenceFailed` after a failed durable commit.
    pub fn correct_owner_claim_with_persistence(
        &mut self,
        id: &ClaimId,
        statement: impl Into<String>,
        kind: ClaimKind,
        persist: impl FnOnce(&ReviewedOwnerContextSnapshot) -> Result<(), String>,
    ) -> Result<(), LabError> {
        if self.session_audience == Some(super::LabSessionAudience::Visitor) {
            return Err(LabError::Runtime(Rt0ReasonCode::AuthScopeDenied));
        }
        let context = self
            .reviewed_owner_context
            .as_mut()
            .ok_or(LabError::InvalidState)?;
        context
            .correct_claim_with_persistence(id, statement, kind, persist)
            .map_err(map_owner_context_error)?;
        self.readiness.reset_for_profile(context.profile())?;
        Ok(())
    }

    /// Restores a previously persisted reviewed owner snapshot between process lifetimes.
    ///
    /// # Errors
    /// Fails closed while a realtime session is active or when the snapshot cannot reconstruct
    /// the same reviewed current state exactly.
    pub fn restore_reviewed_owner_context_snapshot(
        &mut self,
        snapshot: &ReviewedOwnerContextSnapshot,
    ) -> Result<(), LabError> {
        if self
            .session
            .as_ref()
            .is_some_and(|session| session.state() != RealtimeSessionState::Closed)
        {
            return Err(LabError::InvalidState);
        }
        let context =
            ReviewedOwnerContext::from_snapshot(snapshot).map_err(map_owner_context_error)?;
        self.reviewed_owner_context = Some(context);
        Ok(())
    }
}
