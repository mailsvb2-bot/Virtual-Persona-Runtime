//! Bounded, independently revoked RT1 visitor session registry.
//!
//! Internal pre-entry component only. A session never inherits the Owner Lab
//! CSRF, engine mutex, or owner identity. Egress requires a separate final fence.
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;

use crate::rt1_visitor_session::Rt1VisitorSession;
use crate::rt1_workspace_service::Rt1WorkspaceService;

const MAX_ACTIVE_VISITORS: usize = 8;
type StopCallback = Arc<dyn Fn() + Send + Sync>;

struct ManagedVisitor {
    authority: Rt1VisitorSession,
    revoked: AtomicBool,
    stop: StopCallback,
}

impl ManagedVisitor {
    fn revoke(&self) {
        if !self.revoked.swap(true, Ordering::AcqRel) {
            (self.stop)();
        }
    }

    fn authorize(&self) -> Result<(), String> {
        if self.revoked.load(Ordering::Acquire) {
            return Err("visitor session revoked".into());
        }
        if let Err(error) = self.authority.authorize() {
            self.revoke();
            return Err(error);
        }
        if self.revoked.load(Ordering::Acquire) {
            return Err("visitor session revoked".into());
        }
        Ok(())
    }
}

/// Each session has an independent unguessable handle and canonical permit.
/// The stop callback must interrupt provider media without waiting for a
/// worker holding the conversation engine lock.
pub struct VisitorSessionManager {
    workspace: Arc<Rt1WorkspaceService>,
    sessions: Mutex<HashMap<String, Arc<ManagedVisitor>>>,
}

impl VisitorSessionManager {
    #[must_use]
    pub fn new(workspace: Arc<Rt1WorkspaceService>) -> Self {
        Self {
            workspace,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// # Errors
    /// Rejects invalid publication authority, full capacity, or entropy failure.
    pub fn open(
        &self,
        visitor_token: String,
        stop: impl Fn() + Send + Sync + 'static,
    ) -> Result<String, String> {
        // Admission reads canonical durable state. Never create a local session
        // before the publication is authorized.
        let authority = Rt1VisitorSession::admit(Arc::clone(&self.workspace), visitor_token)?;
        let mut secret = [0_u8; 32];
        getrandom::fill(&mut secret).map_err(|_| "visitor session entropy unavailable")?;
        let mut id = String::with_capacity(64);
        for octet in secret {
            use std::fmt::Write;
            write!(&mut id, "{octet:02x}").map_err(|_| "session identifier error")?;
        }
        let session = Arc::new(ManagedVisitor {
            authority,
            revoked: AtomicBool::new(false),
            stop: Arc::new(stop),
        });
        let mut sessions = self.sessions.lock();
        if sessions.len() >= MAX_ACTIVE_VISITORS {
            return Err("visitor session capacity reached".into());
        }
        if sessions.contains_key(&id) {
            return Err("visitor session collision".into());
        }
        // Revalidate AFTER waiting for the registry lock: a pause may have
        // won during local admission. Never grant a stale session.
        session.authorize()?;
        sessions.insert(id.clone(), session);
        Ok(id)
    }

    /// Authorize before and after a pure buffered visitor computation.
    /// This is NOT a provider-egress permission.
    ///
    /// # Errors
    /// Refuses an unknown/revoked session or stale publication authority.
    pub fn compute_buffered<T>(
        &self,
        session_id: &str,
        compute: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let session = self.lookup(session_id)?;
        session.authorize()?;
        let value = session.authority.compute_buffered(compute)?;
        session.authorize()?;
        Ok(value)
    }

    /// Explicit final authorization gate before one protected operation.
    /// The caller must bind provider cancellation to the stop callback.
    ///
    /// # Errors
    /// Denies revoked, missing, or no-longer-authorized sessions.
    pub fn authorize(&self, session_id: &str) -> Result<(), String> {
        self.lookup(session_id)?.authorize()
    }

    /// Immediately invalidate a local session and interrupt its transport.
    ///
    /// # Errors
    /// Returns denial for unknown session handles.
    pub fn close(&self, session_id: &str) -> Result<(), String> {
        let session = self
            .sessions
            .lock()
            .remove(session_id)
            .ok_or_else(|| "visitor session not found".to_owned())?;
        session.revoke();
        Ok(())
    }

    /// Cancel all active visitor transports after owner pause/unpublish.
    /// Call only AFTER the canonical durable owner mutation succeeds.
    pub fn revoke_all(&self) {
        let sessions = {
            let mut active = self.sessions.lock();
            std::mem::take(&mut *active)
        };
        for session in sessions.values() {
            session.revoke();
        }
    }

    #[must_use]
    pub fn active_count(&self) -> usize {
        self.sessions.lock().len()
    }

    fn lookup(&self, session_id: &str) -> Result<Arc<ManagedVisitor>, String> {
        self.sessions
            .lock()
            .get(session_id)
            .cloned()
            .ok_or_else(|| "visitor session not found".to_owned())
    }
}

impl Drop for VisitorSessionManager {
    fn drop(&mut self) {
        self.revoke_all();
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn no_visitor_session_without_canonical_publication() {
        let manager = VisitorSessionManager::new(Arc::new(Rt1WorkspaceService::new()));
        assert!(manager.open("0".repeat(64), || {}).is_err());
        assert_eq!(manager.active_count(), 0);
        assert!(manager.authorize("not-a-session").is_err());
    }
}
