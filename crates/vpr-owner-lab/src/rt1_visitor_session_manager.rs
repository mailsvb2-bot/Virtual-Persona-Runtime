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
        // Insert before the final canonical recheck so concurrent revoke_all
        // can see and stop this admission. Every lookup independently checks
        // authority; registration itself is never an egress grant.
        sessions.insert(id.clone(), Arc::clone(&session));
        drop(sessions);
        // Authorization can revoke and invoke an arbitrary transport callback.
        // Never run that callback under the non-reentrant registry mutex.
        if let Err(error) = session.authorize() {
            // A concurrent close/revoke_all may already have removed it.
            let _ = self.close(&id);
            return Err(error);
        }
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
        // The inner computation can fail because its own post-compute
        // canonical authorization failed. Recheck even on Err so the managed
        // session actively stops any existing provider transport.
        let result = session.authority.compute_buffered(compute);
        finish_buffered(result, || session.authorize())
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
        self.take_for_revocation().revoke();
    }

    /// Drain local registrations while the caller holds the canonical
    /// workspace transaction. Never invoke transport callbacks under that
    /// transaction: a callback may need to re-enter the workspace.
    pub(crate) fn take_for_revocation(&self) -> PendingVisitorRevocations {
        let sessions = {
            let mut active = self.sessions.lock();
            std::mem::take(&mut *active)
        };
        PendingVisitorRevocations {
            sessions: sessions.into_values().collect(),
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

/// In-process transport stops collected during a durable owner mutation.
/// The caller must release the workspace transaction before calling revoke.
pub(crate) struct PendingVisitorRevocations {
    sessions: Vec<Arc<ManagedVisitor>>,
}

impl PendingVisitorRevocations {
    pub(crate) fn revoke(self) {
        for session in self.sessions {
            session.revoke();
        }
    }
}

// Always recheck after a buffered attempt, including the error path.
// An authorization denial takes precedence over a computation error.
fn finish_buffered<T>(
    result: Result<T, String>,
    reauthorize: impl FnOnce() -> Result<(), String>,
) -> Result<T, String> {
    reauthorize()?;
    result
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
    fn drained_transports_stop_once_outside_registry_lock() {
        use std::sync::atomic::AtomicUsize;

        let manager = Arc::new(VisitorSessionManager::new(Arc::new(
            Rt1WorkspaceService::new(),
        )));
        let stops = Arc::new(AtomicUsize::new(0));
        let callback_manager = Arc::clone(&manager);
        let callback_stops = Arc::clone(&stops);
        let session = Arc::new(ManagedVisitor {
            authority: Rt1VisitorSession::inert_for_revocation_test(),
            revoked: AtomicBool::new(false),
            stop: Arc::new(move || {
                // A stop callback may re-enter the registry; no mutex is held.
                assert_eq!(callback_manager.active_count(), 0);
                callback_stops.fetch_add(1, Ordering::SeqCst);
            }),
        });
        manager
            .sessions
            .lock()
            .insert("old-publication".into(), Arc::clone(&session));

        let pending = manager.take_for_revocation();
        assert_eq!(manager.active_count(), 0);
        assert!(!session.revoked.load(Ordering::Acquire));
        pending.revoke();
        assert!(session.revoked.load(Ordering::Acquire));
        session.revoke();
        assert_eq!(stops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn registry_drain_does_not_stop_sessions_added_after_drain() {
        use std::sync::atomic::AtomicUsize;

        let manager = VisitorSessionManager::new(Arc::new(Rt1WorkspaceService::new()));
        let old_stops = Arc::new(AtomicUsize::new(0));
        let new_stops = Arc::new(AtomicUsize::new(0));
        let old_counter = Arc::clone(&old_stops);
        let new_counter = Arc::clone(&new_stops);
        let old_session = Arc::new(ManagedVisitor {
            authority: Rt1VisitorSession::inert_for_revocation_test(),
            revoked: AtomicBool::new(false),
            stop: Arc::new(move || {
                old_counter.fetch_add(1, Ordering::SeqCst);
            }),
        });
        let new_session = Arc::new(ManagedVisitor {
            authority: Rt1VisitorSession::inert_for_revocation_test(),
            revoked: AtomicBool::new(false),
            stop: Arc::new(move || {
                new_counter.fetch_add(1, Ordering::SeqCst);
            }),
        });
        manager.sessions.lock().insert("old".into(), old_session);
        let pending = manager.take_for_revocation();
        manager.sessions.lock().insert("new".into(), new_session);
        pending.revoke();
        assert_eq!(old_stops.load(Ordering::SeqCst), 1);
        assert_eq!(new_stops.load(Ordering::SeqCst), 0);
        assert_eq!(manager.active_count(), 1);
        manager.revoke_all();
        assert_eq!(new_stops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn no_visitor_session_without_canonical_publication() {
        let manager = VisitorSessionManager::new(Arc::new(Rt1WorkspaceService::new()));
        assert!(manager.open("0".repeat(64), || {}).is_err());
        assert_eq!(manager.active_count(), 0);
        assert!(manager.authorize("not-a-session").is_err());
    }

    #[test]
    fn failed_buffered_computation_still_checks_revocation() {
        let rechecked = AtomicBool::new(false);
        let result: Result<(), String> =
            finish_buffered(Err("buffered computation failed".into()), || {
                rechecked.store(true, Ordering::SeqCst);
                Err("visitor session revoked".into())
            });
        assert!(rechecked.load(Ordering::SeqCst));
        assert_eq!(result.unwrap_err(), "visitor session revoked");
    }

    #[test]
    fn failed_computation_preserves_error_when_authority_is_valid() {
        let result: Result<(), String> =
            finish_buffered(Err("computation failed".into()), || Ok(()));
        assert_eq!(result.unwrap_err(), "computation failed");
    }

    #[test]
    fn successful_computation_cannot_bypass_final_revocation() {
        let result = finish_buffered(Ok("unreleased"), || Err("visitor session revoked".into()));
        assert_eq!(result.unwrap_err(), "visitor session revoked");
    }
}
