//! Internal visitor authorization handle for future independently isolated sessions.
//! Never grants direct media/provider egress or owner privileges.
use std::sync::Arc;
use vpr_policy::publication_lifecycle::VisitorPermit;

use crate::rt1_workspace_service::Rt1WorkspaceService;

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
        now_millis: u64,
    ) -> Result<Self, String> {
        let permit = workspace.admit_visitor_turn(&bearer_token, now_millis)?;
        Ok(Self {
            workspace,
            bearer_token,
            permit,
        })
    }

    /// # Errors
    /// Rejects revoked, expired, or version-stale authority.
    pub fn authorize(&self, now_millis: u64) -> Result<(), String> {
        self.workspace
            .authorize_visitor(&self.bearer_token, &self.permit, now_millis)
    }

    /// Pure computation, not permission for external output or media.
    /// # Errors
    /// Denies failed computation or authority changed during processing.
    pub fn compute_buffered<T>(
        &self,
        compute: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.workspace.guarded_buffered_visitor_output(
            &self.bearer_token,
            &self.permit,
            compute,
        )
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

/// Provider transport contract for an independently authorized visitor.
/// An implementation MUST fence every send using current canonical authority;
/// implementing this trait alone does not enable public RT1 media.
pub trait Rt1VisitorMediaTransport {
    type Error;

    fn open(&mut self) -> Result<(), Self::Error>;
    fn send_audio(&mut self, pcm: &[u8]) -> Result<(), Self::Error>;
    fn send_video(&mut self, frame: &[u8]) -> Result<(), Self::Error>;
    fn stop(&mut self) -> Result<(), Self::Error>;
    fn close(&mut self) -> Result<(), Self::Error>;
}
