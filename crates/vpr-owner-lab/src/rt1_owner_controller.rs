//! Internal owner command boundary. Not exposed as HTTP until RT1 release entry.
use crate::rt1_workspace_service::{Rt1WorkspaceService, WorkspaceStatus};

pub struct Rt1OwnerController {
    workspace: Rt1WorkspaceService,
}

impl Default for Rt1OwnerController {
    fn default() -> Self {
        Self::new()
    }
}

impl Rt1OwnerController {
    #[must_use]
    pub fn new() -> Self {
        Self {
            workspace: Rt1WorkspaceService::new(),
        }
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
    pub fn pause(&self, owner_token: &str) -> Result<(), String> {
        self.workspace.pause(owner_token)
    }

    /// # Errors
    /// Refuses invalid owner or failed durable commit.
    pub fn unpublish(&self, owner_token: &str) -> Result<(), String> {
        self.workspace.unpublish(owner_token)
    }
}
