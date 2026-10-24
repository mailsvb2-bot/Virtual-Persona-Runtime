//! Internal owner command boundary. Not exposed as HTTP until RT1 release entry.
use std::sync::{Arc, OnceLock};

use crate::rt1_visitor_session_manager::VisitorSessionManager;
use crate::rt1_workspace_service::{Rt1WorkspaceService, WorkspaceStatus};

// In-process owner commands must never maintain separate visitor registries:
/// otherwise an owner pause through one controller would miss other sessions.
static OWNER_VISITOR_RUNTIME: OnceLock<(Arc<Rt1WorkspaceService>, Arc<VisitorSessionManager>)> =
    OnceLock::new();

pub struct Rt1OwnerController {
    workspace: Arc<Rt1WorkspaceService>,
    visitors: Arc<VisitorSessionManager>,
}

impl Default for Rt1OwnerController {
    fn default() -> Self {
        Self::new()
    }
}

impl Rt1OwnerController {
    #[must_use]
    pub fn new() -> Self {
        let (workspace, visitors) = OWNER_VISITOR_RUNTIME.get_or_init(|| {
            let workspace = Arc::new(Rt1WorkspaceService::new());
            let visitors = Arc::new(VisitorSessionManager::new(Arc::clone(&workspace)));
            (workspace, visitors)
        });
        Self {
            workspace: Arc::clone(workspace),
            visitors: Arc::clone(visitors),
        }
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
        self.workspace.pause(owner_token)?;
        self.visitors.revoke_all();
        Ok(())
    }

    /// # Errors
    /// Refuses invalid owner or failed durable commit.
    /// After the durable mutation, registered visitor transports are stopped.
    pub fn unpublish(&self, owner_token: &str) -> Result<(), String> {
        self.workspace.unpublish(owner_token)?;
        self.visitors.revoke_all();
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
        assert!(Arc::ptr_eq(&first, &independent_controller.visitor_sessions()));
        assert_eq!(first.active_count(), 0);
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
