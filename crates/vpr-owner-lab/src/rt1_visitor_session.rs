//! Internal visitor authorization handle for future independently isolated sessions.
//! Never grants direct media/provider egress or owner privileges.
use std::sync::Arc;
use vpr_policy::publication_lifecycle::VisitorPermit;

use crate::rt1_workspace_service::{Rt1WorkspaceService, server_now_millis};

pub struct Rt1VisitorSession {
    workspace: Arc<Rt1WorkspaceService>,
    bearer_token: String,
    permit: VisitorPermit,
}

impl Rt1VisitorSession {
    /// # Errors
    /// Denies unverified or stale publication authority.
    pub fn admit(
        workspace: Arc<Rt1WorkspaceService>,
        bearer_token: String,
    ) -> Result<Self, String> {
        let permit = workspace.admit_visitor_turn(&bearer_token, server_now_millis()?)?;
        Ok(Self {
            workspace,
            bearer_token,
            permit,
        })
    }

    /// Test-only inert authority. It cannot authenticate against the durable
    /// store and must never be used as a real visitor admission.
    #[cfg(test)]
    pub(crate) fn inert_for_revocation_test() -> Self {
        use vpr_policy::publication_lifecycle::CanonicalPublication;
        let mut publication =
            CanonicalPublication::new("owner", "persona", "publication", 1).unwrap();
        publication.publish("owner", 1, true).unwrap();
        Self {
            workspace: Arc::new(Rt1WorkspaceService::new()),
            bearer_token: "not-an-admitted-visitor".into(),
            permit: publication.visitor_lease(true).unwrap(),
        }
    }

    /// # Errors
    /// Rejects revoked, expired, or version-stale authority.
    pub fn authorize(&self) -> Result<(), String> {
        self.workspace
            .authorize_visitor(&self.bearer_token, &self.permit, server_now_millis()?)
    }

    /// Pure computation, not permission for external output or media.
    /// # Errors
    /// Denies failed computation or authority changed during processing.
    pub fn compute_buffered<T>(
        &self,
        compute: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.workspace
            .guarded_buffered_visitor_output(&self.bearer_token, &self.permit, compute)
    }

    /// Bounded local queue only. Consumer must re-authorize final egress.
    /// # Errors
    /// Rejects denied authority or full/disconnected queue.
    pub fn enqueue_fragment<T>(
        &self,
        sender: &std::sync::mpsc::SyncSender<T>,
        fragment: T,
    ) -> Result<(), String> {
        self.workspace.enqueue_authorized_visitor_fragment(
            &self.bearer_token,
            &self.permit,
            sender,
            fragment,
        )
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn visitor_admission_fails_closed_without_canonical_persona_store() {
        let workspace = Arc::new(Rt1WorkspaceService::new());
        assert!(Rt1VisitorSession::admit(workspace, "0".repeat(64)).is_err());
    }
}
