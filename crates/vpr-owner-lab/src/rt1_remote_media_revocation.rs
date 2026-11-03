//! Bridges canonical owner-driven visitor revocation to the worker's durable
//! operation-id STOP protocol. No independent authority or media session owner.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use vpr_provider_local_open_source::operation_client::RemoteOperationClient;

use crate::rt1_visitor_session_manager::VisitorSessionManager;

/// A failed network STOP remains visibly unconfirmed, never equivalent to
/// revocation completing at the remote audio/video sender.
pub struct Rt1RemoteMediaRevocation {
    worker: RemoteOperationClient,
    operation_id: String,
    generation: u64,
    revoked: AtomicBool,
    confirmed: AtomicBool,
    stop_gate: Mutex<()>,
    /// CREATE can return after owner revoke/STOP. Correlate both outcomes to
    /// the exact remote sender, not just the operation/generation envelope.
    created_session_id: Mutex<Option<String>>,
    confirmed_session_id: Mutex<Option<String>>,
    /// A conflicting late CREATE identity requires external reconciliation;
    /// repeated STOP for the first sender must never clear this quarantine.
    identity_conflict: AtomicBool,
}

impl Rt1RemoteMediaRevocation {
    /// # Errors
    /// Rejects invalid operation identity before any owner/visitor registration.
    pub fn new(
        worker: RemoteOperationClient,
        operation_id: String,
        generation: u64,
    ) -> Result<Self, String> {
        if generation == 0
            || operation_id.is_empty()
            || operation_id.len() > 128
            || !operation_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err("invalid remote operation identity".into());
        }
        Ok(Self {
            worker,
            operation_id,
            generation,
            revoked: AtomicBool::new(false),
            confirmed: AtomicBool::new(false),
            stop_gate: Mutex::new(()),
            created_session_id: Mutex::new(None),
            confirmed_session_id: Mutex::new(None),
            identity_conflict: AtomicBool::new(false),
        })
    }

    /// Bind a late CREATE reply to the operation before publishing its SDP.
    /// A STOP receipt for a different sender cannot authorize that CREATE.
    /// The STOP gate serializes this binding against remote cancellation.
    ///
    /// # Errors
    /// Refuses conflicting identities, including STOP-before-CREATE races.
    pub fn record_created_session(&self, session_id: &str) -> Result<(), String> {
        if session_id.trim().is_empty() {
            return Err("RT1_REMOTE_SESSION_ID_INVALID".into());
        }
        let _serial = self.stop_gate.lock();
        let mut created = self.created_session_id.lock();
        if created
            .as_deref()
            .is_some_and(|previous| previous != session_id)
            || self
                .confirmed_session_id
                .lock()
                .as_deref()
                .is_some_and(|confirmed| confirmed != session_id)
        {
            self.identity_conflict.store(true, Ordering::Release);
            self.revoked.store(true, Ordering::Release);
            self.confirmed.store(false, Ordering::Release);
            return Err("RT1_REMOTE_SESSION_ID_CONFLICT".into());
        }
        *created = Some(session_id.to_owned());
        Ok(())
    }

    /// Invoked by the existing canonical visitor stop callback, after the
    /// owner publication transaction is released. No engine mutex is held.
    /// An uncertain outcome stays unconfirmed and can be reconciled later.
    ///
    /// # Errors
    /// No remote sender STOP receipt, stale generation or transport timeout.
    pub fn revoke_confirmed(&self) -> Result<(), String> {
        self.revoked.store(true, Ordering::Release);
        let _serial = self.stop_gate.lock();
        if self.confirmed.load(Ordering::Acquire) {
            return Ok(());
        }
        self.cancel_locked()
    }

    // Called with stop_gate held. Both owner revoke and failed SDP answer
    // require the same independent audio/video STOP receipt.
    fn cancel_locked(&self) -> Result<(), String> {
        if self.identity_conflict.load(Ordering::Acquire) {
            return Err("RT1_REMOTE_SESSION_ID_CONFLICT".into());
        }
        if self.confirmed.load(Ordering::Acquire) {
            return Ok(());
        }
        let stopped_session = self
            .worker
            .cancel_confirmed(&self.operation_id, self.generation)
            .map_err(|_| "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED".to_owned())?;
        if self
            .created_session_id
            .lock()
            .as_deref()
            .is_some_and(|created| created != stopped_session)
        {
            // The worker acknowledged STOP for a different sender. Preserve
            // quarantine; never advertise confirmed audio/video termination.
            return Err("RT1_REMOTE_MEDIA_STOP_IDENTITY_MISMATCH".into());
        }
        *self.confirmed_session_id.lock() = Some(stopped_session);
        self.confirmed.store(true, Ordering::Release);
        Ok(())
    }

    /// Submit SDP only while the operation is not revoked.
    /// # Errors
    /// Denies revoked operations and remote signaling failures.
    pub fn submit_answer(&self, sdp: &str) -> Result<(), String> {
        let _serial = self.stop_gate.lock();
        if self.revoked.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        if self.worker.answer(&self.operation_id, sdp).is_err() {
            // A rejected or lost SDP receipt does not prove the provider
            // never accepted it. Quarantine and compensate under the same
            // signaling/STOP gate, preventing any second answer.
            self.revoked.store(true, Ordering::Release);
            self.cancel_locked()?;
            return Err("RT1_REMOTE_ANSWER_UNCONFIRMED".to_owned());
        }
        // Owner revoke marks this flag before acquiring stop_gate. If it
        // arrived during the in-flight HTTP request, a late provider ACK must
        // not be exposed to the visitor as successful authorization.
        if self.revoked.load(Ordering::Acquire) {
            // The owner may have revoked authority while the provider was
            // accepting SDP. Complete the compensating STOP *before* the
            // signaling caller returns: a delayed owner callback must not be
            // the only path to shutting down an already accepted answer.
            // Keep the operation quarantined when either sender is uncertain.
            self.cancel_locked()?;
            return Err("RT1_VISITOR_REVOKED".into());
        }
        Ok(())
    }

    #[must_use]
    pub fn is_revoked(&self) -> bool {
        self.revoked.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn is_stop_confirmed(&self) -> bool {
        self.confirmed.load(Ordering::Acquire)
    }
}

impl VisitorSessionManager {
    /// Register exactly one remote operation STOP with the existing visitor
    /// authority. Caller must retain the returned bridge to inspect or retry
    /// an uncertain remote shutdown. No new permission model is introduced.
    ///
    /// # Errors
    /// Denies visitor admission unless the current Persona is published.
    pub fn open_with_remote_operation(
        &self,
        visitor_token: String,
        remote: Arc<Rt1RemoteMediaRevocation>,
    ) -> Result<String, String> {
        self.open(visitor_token, move || {
            // A failed STOP is not a success: remote.confirmed remains false.
            // Worker operation records remain durable and must be reconciled.
            let _ = remote.revoke_confirmed();
        })
    }
}

#[cfg(test)]
#[path = "rt1_remote_media_revocation_tests.rs"]
mod tests;
