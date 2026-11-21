//! Internal owner command boundary. Not exposed as HTTP until RT1 release entry.
use std::sync::{Arc, OnceLock};

use crate::rt1_owner_enrollment::Rt1OwnerEnrollments;
use crate::rt1_visitor_session_manager::VisitorSessionManager;
use crate::rt1_workspace_service::{Rt1WorkspaceService, WorkspaceStatus};

// In-process owner commands must never maintain separate visitor registries:
/// otherwise an owner pause through one controller would miss other sessions.
static OWNER_VISITOR_RUNTIME: OnceLock<(
    Arc<Rt1WorkspaceService>,
    Arc<VisitorSessionManager>,
    Arc<Rt1OwnerEnrollments>,
)> =
    OnceLock::new();

pub struct Rt1OwnerController {
    workspace: Arc<Rt1WorkspaceService>,
    visitors: Arc<VisitorSessionManager>,
    enrollments: Arc<Rt1OwnerEnrollments>,
}

impl Default for Rt1OwnerController {
    fn default() -> Self {
        Self::new()
    }
}

impl Rt1OwnerController {
    #[must_use]
    pub fn new() -> Self {
        let (workspace, visitors, enrollments) = OWNER_VISITOR_RUNTIME.get_or_init(|| {
            let workspace = Arc::new(Rt1WorkspaceService::new());
            let visitors = Arc::new(VisitorSessionManager::new(Arc::clone(&workspace)));
            (workspace, visitors, Arc::new(Rt1OwnerEnrollments::default()))
        });
        Self {
            workspace: Arc::clone(workspace),
            visitors: Arc::clone(visitors),
            enrollments: Arc::clone(enrollments),
        }
    }

    /// Read the reviewed Persona identity from the SAME durable workspace
    /// used to admit and revoke visitor sessions.
    ///
    /// # Errors
    /// Denies missing reviewed Persona or invalid durable workspace state.
    pub fn current_persona_identity(&self) -> Result<(String, u64), String> {
        self.workspace.current_persona_identity()
    }

    /// Identify the exact owner and reviewed Persona from a server-issued
    /// visitor bearer, before selecting any external provider credentials.
    ///
    /// # Errors
    /// Rejects invalid, stale or revoked visitor authority.
    pub fn owner_binding_for_visitor(
        &self,
        bearer_token: &str,
    ) -> Result<crate::VisitorOwnerBinding, String> {
        self.workspace.owner_binding_for_visitor(bearer_token)
    }

    /// The legacy provider stack is not yet tenant-scoped.
    ///
    /// # Errors
    /// Rejects unknown, revoked, or non-primary visitor publication authority.
    pub fn require_legacy_provider_owner(&self, visitor_token: &str) -> Result<(), String> {
        self.workspace.require_legacy_provider_owner(visitor_token)
    }

    /// Return an exact canonical owner and private provider profile only
    /// after authenticating the visitor against the active publication.
    ///
    /// # Errors
    /// Denies revoked visitors, missing owner profiles and invalid credentials.
    pub fn owner_provider_binding_for_visitor(
        &self,
        visitor_token: &str,
    ) -> Result<
        (
            crate::VisitorOwnerBinding,
            Option<crate::OwnerProviderProfile>,
        ),
        String,
    > {
        let owner = self.workspace.owner_binding_for_visitor(visitor_token)?;
        let scoped = crate::rt1_owner_provider_profile::profile_for_owner(&owner.owner_id)?;
        Ok((owner, scoped))
    }

    /// Authenticate the owner before opening its private provider profile.
    ///
    /// # Errors
    /// Denies invalid owner tokens or unavailable private profiles.
    pub fn owner_provider_binding_for_owner(
        &self,
        owner_token: &str,
    ) -> Result<(WorkspaceStatus, Option<crate::OwnerProviderProfile>), String> {
        let owner = self.workspace.owner_status(owner_token)?;
        let scoped = crate::rt1_owner_provider_profile::profile_for_owner(&owner.owner_id)?;
        Ok((owner, scoped))
    }

    /// Start a private, bounded owner interview. The bearer is random and
    /// does not grant owner/publication authority before review and commit.
    ///
    /// # Errors
    /// Rejects invalid Persona names, resource exhaustion and entropy failures.
    pub fn begin_owner_registration(
        &self,
        persona_id: String,
    ) -> Result<(String, crate::OwnerCaptureSnapshot), String> {
        self.enrollments.begin(persona_id)
    }

    /// # Errors
    /// Denies invalid or expired enrollment credentials.
    pub fn owner_registration_status(
        &self,
        token: &str,
    ) -> Result<crate::OwnerCaptureSnapshot, String> {
        self.enrollments.snapshot(token)
    }

    /// # Errors
    /// Denies unauthenticated or out-of-order interview answers.
    pub fn owner_registration_answer(
        &self,
        token: &str,
        answer: String,
    ) -> Result<crate::OwnerCaptureSnapshot, String> {
        self.enrollments.answer(token, answer)
    }

    /// # Errors
    /// Rejects incomplete owner interviews.
    pub fn owner_registration_finish(
        &self,
        token: &str,
    ) -> Result<crate::OwnerCaptureSnapshot, String> {
        self.enrollments.finish(token)
    }

    /// # Errors
    /// Rejects unreviewed or unknown captured claims.
    pub fn owner_registration_approve(
        &self,
        token: &str,
        claim_id: &str,
    ) -> Result<crate::OwnerCaptureSnapshot, String> {
        self.enrollments.approve(token, claim_id)
    }

    /// Commit a new owner and reviewed Persona in the ONLY canonical store.
    ///
    /// # Errors
    /// Rejects incomplete reviews, duplicate Persona, unavailable storage,
    /// or invalid enrollment credentials.
    pub fn complete_owner_registration(
        &self,
        token: &str,
    ) -> Result<(String, String), String> {
        self.enrollments.complete(token, &self.workspace)
    }

    /// Share the exact in-process visitor registry with the eventual runtime.
    /// All owner revoke and visitor admission calls must use this same instance.
    #[must_use]
    pub fn visitor_sessions(&self) -> Arc<VisitorSessionManager> {
        Arc::clone(&self.visitors)
    }

    /// # Errors
    /// Refuses duplicate provisioning or unavailable canonical storage.
    pub fn provision_owner(&self) -> Result<(String, String), String> {
        self.workspace.provision_owner()
    }

    /// # Errors
    /// Denies invalid owner credentials or missing workspace.
    pub fn status(&self, owner_token: &str) -> Result<WorkspaceStatus, String> {
        self.workspace.owner_status(owner_token)
    }

    /// The caller supplies a server-owned readiness verifier. No browser boolean
    /// can authorize publication. The service rechecks the exact `PersonaVersion`.
    ///
    /// # Errors
    /// Refuses stale Persona, failed readiness, or unauthorized publication.
    pub fn publish(
        &self,
        owner_token: &str,
        expected_persona_version: u64,
        verify_readiness: impl FnOnce() -> Result<(), String>,
    ) -> Result<String, String> {
        let status = self.workspace.owner_status(owner_token)?;
        if status.persona_version != expected_persona_version {
            return Err("STALE_PERSONA_VERSION".into());
        }
        verify_readiness()?;
        self.workspace
            .publish(owner_token, expected_persona_version, true)
    }

    /// # Errors
    /// Refuses invalid owner or failed durable commit.
    /// After the durable mutation, registered visitor transports are stopped.
    pub fn pause(&self, owner_token: &str) -> Result<(), String> {
        let pending = self
            .workspace
            .pause_and_collect_revocations(owner_token, &self.visitors)?;
        pending.revoke();
        Ok(())
    }

    /// # Errors
    /// Refuses invalid owner or failed durable commit.
    /// After the durable mutation, registered visitor transports are stopped.
    pub fn unpublish(&self, owner_token: &str) -> Result<(), String> {
        let pending = self
            .workspace
            .unpublish_and_collect_revocations(owner_token, &self.visitors)?;
        pending.revoke();
        Ok(())
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn owner_controller_exposes_same_visitor_registry_handle() {
        let controller = Rt1OwnerController::new();
        let first = controller.visitor_sessions();
        let second = controller.visitor_sessions();
        assert!(Arc::ptr_eq(&first, &second));
        let independent_controller = Rt1OwnerController::new();
        assert!(Arc::ptr_eq(
            &first,
            &independent_controller.visitor_sessions()
        ));
    }

    #[test]
    fn unauthorized_owner_cannot_trigger_readiness_or_publish() {
        let controller = Rt1OwnerController::new();
        let token = "0".repeat(64);
        let mut readiness_called = false;
        let result = controller.publish(&token, 1, || {
            readiness_called = true;
            Ok(())
        });
        assert!(result.is_err());
        assert!(!readiness_called);
        assert!(controller.status(&token).is_err());
        assert!(controller.pause(&token).is_err());
        assert!(controller.unpublish(&token).is_err());
    }
}
