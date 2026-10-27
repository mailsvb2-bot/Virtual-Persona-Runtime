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
    fn revoke_does_not_wait_for_stalled_provider_worker() {
        use std::sync::atomic::AtomicUsize;
        use std::sync::mpsc;
        use std::time::Duration;

        let manager = Arc::new(VisitorSessionManager::new(Arc::new(
            Rt1WorkspaceService::new(),
        )));
        // Simulate an unresponsive provider worker holding its conversation
        // engine mutex. The canonical registry must never acquire that mutex
        // while revoking admission or running the transport stop callback.
        let engine = Arc::new(Mutex::new(()));
        let (worker_locked_tx, worker_locked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker_engine = Arc::clone(&engine);
        let worker = std::thread::spawn(move || {
            let _held = worker_engine.lock();
            worker_locked_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        worker_locked_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap();

        let stops = Arc::new(AtomicUsize::new(0));
        let stop_count = Arc::clone(&stops);
        manager.sessions.lock().insert(
            "stalled".into(),
            Arc::new(ManagedVisitor {
                authority: Rt1VisitorSession::inert_for_revocation_test(),
                revoked: AtomicBool::new(false),
                stop: Arc::new(move || {
                    stop_count.fetch_add(1, Ordering::SeqCst);
                }),
            }),
        );

        let (revoked_tx, revoked_rx) = mpsc::channel();
        let revoking_manager = Arc::clone(&manager);
        let revoker = std::thread::spawn(move || {
            revoking_manager.revoke_all();
            revoked_tx.send(()).unwrap();
        });
        // Unstick the fake worker even on failure so the test cannot hang CI.
        let promptly_revoked = revoked_rx.recv_timeout(Duration::from_secs(2)).is_ok();
        release_tx.send(()).unwrap();
        worker.join().unwrap();
        revoker.join().unwrap();
        assert!(
            promptly_revoked,
            "visitor revocation blocked on provider worker"
        );
        assert_eq!(manager.active_count(), 0);
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

    #[test]
    fn owner_revocation_dispatches_actual_remote_operation_cancel_http() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::atomic::AtomicUsize;
        use std::time::Duration;
        use vpr_provider_local_open_source::operation_client::RemoteOperationClient;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let request_count = Arc::new(AtomicUsize::new(0));
        let server_count = Arc::clone(&request_count);
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0_u8; 4096];
            loop {
                let n = socket.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..n]);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&bytes);
            assert!(request.starts_with("DELETE /v1/avatar/operations/owner-revoked-1 "));
            server_count.fetch_add(1, Ordering::SeqCst);
            let reply = r#"{"operation_id":"owner-revoked-1","generation":1,"session_id":"remote-1","audio_stopped":true,"video_stopped":true}"#;
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                reply.len(),
                reply
            )
            .unwrap();
        });

        let client =
            RemoteOperationClient::new(&endpoint, "worker-secret", Duration::from_secs(2)).unwrap();
        let manager = VisitorSessionManager::new(Arc::new(Rt1WorkspaceService::new()));
        let remote = Arc::new(
            crate::rt1_remote_media_revocation::Rt1RemoteMediaRevocation::new(
                client,
                "owner-revoked-1".into(),
                1,
            )
            .unwrap(),
        );
        let callback_remote = Arc::clone(&remote);
        manager.sessions.lock().insert(
            "owner-session".into(),
            Arc::new(ManagedVisitor {
                authority: Rt1VisitorSession::inert_for_revocation_test(),
                revoked: AtomicBool::new(false),
                stop: Arc::new(move || {
                    let _ = callback_remote.revoke_confirmed();
                }),
            }),
        );
        manager.revoke_all();
        server.join().unwrap();
        assert_eq!(request_count.load(Ordering::SeqCst), 1);
        assert!(remote.is_revoked());
        assert!(remote.is_stop_confirmed());
        assert_eq!(manager.active_count(), 0);
        manager.revoke_all();
        assert_eq!(request_count.load(Ordering::SeqCst), 1);
    }
}
