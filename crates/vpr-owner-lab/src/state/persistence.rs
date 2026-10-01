use vpr_domain::{ClaimId, ClaimKind, RealtimeSessionState, Rt0ReasonCode};

use crate::owner_context::{
    DurableReviewedOwnerContextSnapshot, ReviewedOwnerContext, ReviewedOwnerContextSnapshot,
};

use super::{LabError, OwnerLabEngine, map_owner_context_error};

impl OwnerLabEngine {
    /// Returns an owner-only snapshot of the current reviewed claim revisions.
    /// Previous revisions are intentionally not copied into this browser-facing view.
    ///
    /// # Errors
    /// Returns `InvalidState` until explicit owner review has completed.
    pub fn reviewed_owner_context_snapshot(
        &self,
    ) -> Result<ReviewedOwnerContextSnapshot, LabError> {
        if self.session_audience == Some(super::LabSessionAudience::Visitor) {
            return Err(LabError::Runtime(Rt0ReasonCode::AuthScopeDenied));
        }
        self.reviewed_owner_context
            .as_ref()
            .map(ReviewedOwnerContext::snapshot)
            .ok_or(LabError::InvalidState)
    }

    /// Corrects one reviewed owner claim and atomically commits the exact durable history.
    ///
    /// # Errors
    /// Returns the canonical correction error, or `PersistenceFailed` when the durable commit fails.
    pub fn correct_owner_claim_persisted(
        &mut self,
        id: &ClaimId,
        statement: impl Into<String>,
        kind: ClaimKind,
    ) -> Result<(), LabError> {
        self.correct_owner_claim_with_persistence(
            id,
            statement,
            kind,
            crate::persona_persistence::save_reviewed_persona,
        )
    }
    /// Corrects one reviewed owner claim and commits the resulting snapshot atomically with
    /// durable persistence. The domain layer rolls back only the touched claim revision and
    /// `PersonaVersion` if persistence fails; readiness is reset only after a successful commit.
    ///
    /// # Errors
    /// Returns the canonical correction error, or `PersistenceFailed` after a failed durable commit.
    pub(crate) fn correct_owner_claim_with_persistence(
        &mut self,
        id: &ClaimId,
        statement: impl Into<String>,
        kind: ClaimKind,
        persist: impl FnOnce(&DurableReviewedOwnerContextSnapshot) -> Result<(), String>,
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
    pub(crate) fn restore_reviewed_owner_context(
        &mut self,
        snapshot: &DurableReviewedOwnerContextSnapshot,
    ) -> Result<(), LabError> {
        if self
            .session
            .as_ref()
            .is_some_and(|session| session.state() != RealtimeSessionState::Closed)
        {
            return Err(LabError::InvalidState);
        }
        let context = ReviewedOwnerContext::from_durable_snapshot(snapshot)
            .map_err(map_owner_context_error)?;
        self.reviewed_owner_context = Some(context);
        Ok(())
    }
}
