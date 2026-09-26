use vpr_domain::RealtimeSessionState;

use crate::owner_context::{ReviewedOwnerContext, ReviewedOwnerContextSnapshot};

use super::{LabError, OwnerLabEngine, map_owner_context_error};

impl OwnerLabEngine {
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
