//! Bounded, independently revoked RT1 visitor session registry.
//!
//! Internal pre-entry component only. A session never inherits the Owner Lab
//! CSRF, engine mutex, or owner identity. Egress requires a separate final fence.
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;

use crate::rt1_visitor_session::Rt1VisitorSession;
#[path = "rt1_visitor_egress.rs"]
mod egress;
use crate::rt1_workspace_service::Rt1WorkspaceService;
pub(crate) use egress::VisitorEgressPermit;
use egress::VisitorEgressState;

const MAX_ACTIVE_VISITORS: usize = 8;
type StopCallback = Arc<dyn Fn() + Send + Sync>;

struct ManagedVisitor {
    authority: Rt1VisitorSession,
    revoked: AtomicBool,
    stopped: AtomicBool,
    /// Pure runtime authority cancellation; NEVER provider/network I/O.
    fast_revoke: StopCallback,
    stop: StopCallback,
    egress: Arc<VisitorEgressState>,
}

impl ManagedVisitor {
    fn mark_revoked(&self) {
        if !self.revoked.swap(true, Ordering::AcqRel) {
            // The caller may hold the canonical publication transaction.
            // Only invalidate runtime permits/turns; network STOP runs later.
            (self.fast_revoke)();
        }
    }

    fn revoke(&self) {
        self.mark_revoked();
        if !self.stopped.swap(true, Ordering::AcqRel) {
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
        self.open_with_fast_revoke(visitor_token, || {}, stop)
    }

    /// Bind the canonical runtime permit revocation to the same publication
    /// transaction as owner pause/unpublish. The fast callback MUST NOT
    /// perform provider I/O or acquire the conversation engine mutex.
    ///
    /// # Errors
    /// Refuses invalid publication, full capacity, or entropy failure.
    pub(crate) fn open_with_fast_revoke(
        &self,
        visitor_token: String,
        fast_revoke: impl Fn() + Send + Sync + 'static,
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
            stopped: AtomicBool::new(false),
            fast_revoke: Arc::new(fast_revoke),
            stop: Arc::new(stop),
            egress: Arc::new(VisitorEgressState::default()),
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

    /// Queue one bounded media fragment under the SAME canonical workspace
    /// publication transaction as pause/unpublish. No provider I/O occurs
    /// while this transaction is held. The consumer still needs a final
    /// revocation-serialized egress/STOP gate before any external delivery.
    ///
    /// # Errors
    /// Refuses unknown, revoked or stale visitors and full queues.
    pub fn enqueue_media_fragment<T>(
        &self,
        session_id: &str,
        sender: &std::sync::mpsc::SyncSender<T>,
        fragment: T,
    ) -> Result<(), String> {
        let session = self.lookup(session_id)?;
        if session.revoked.load(Ordering::Acquire) {
            return Err("visitor session revoked".into());
        }
        session.authority.enqueue_fragment(sender, fragment)
    }

    /// Register the visitor provider turn under the canonical publication
    /// transaction. This is the same owner/visitor admission boundary used
    /// by the existing runtime: it does not create a second engine.
    /// External provider I/O MUST run after this method returns, through
    /// the canonical interruptible runtime turn.
    ///
    /// # Errors
    /// Denies revoked or stale publication before registering the lease.
    pub(crate) fn begin_provider_egress(
        &self,
        session_id: &str,
    ) -> Result<VisitorEgressPermit, String> {
        let session = self.lookup(session_id)?;
        session
            .authority
            .admit_provider_egress(|| session.egress.admit(&session.revoked))
    }

    /// Wait for the visitor-scoped publication-to-provider lease to drain.
    /// This is a local quiescence check, never a remote RTP STOP receipt.
    pub(crate) fn await_provider_quiescence(&self, session_id: &str, timeout: Duration) -> bool {
        self.lookup(session_id)
            .is_ok_and(|session| session.egress.quiescent(timeout))
    }

    /// Explicit final authorization gate before one protected operation.
    /// The caller must bind provider cancellation to the stop callback.
    ///
    /// # Errors
    /// Denies revoked, missing, or no-longer-authorized sessions.
    pub fn authorize(&self, session_id: &str) -> Result<(), String> {
        self.lookup(session_id)?.authorize()
    }

    /// # Errors
    /// Denies an engine whose reviewed identity or version differs from the
    /// server-admitted publication, including after correction or republish.
    pub(crate) fn authorize_persona(
        &self,
        session_id: &str,
        persona_id: &str,
        version: u64,
    ) -> Result<(), String> {
        let session = self.lookup(session_id)?;
        session.authorize()?;
        session.authority.authorize_persona(persona_id, version)
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
        let sessions: Vec<_> = sessions.into_values().collect();
        // Called under the canonical publication transaction: invalidate
        // provider permits BEFORE a concurrent visitor can start egress.
        // Never run network STOP while this transaction is held.
        for session in &sessions {
            session.mark_revoked();
        }
        PendingVisitorRevocations { sessions }
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
#[path = "rt1_visitor_session_manager_tests.rs"]
mod tests;
