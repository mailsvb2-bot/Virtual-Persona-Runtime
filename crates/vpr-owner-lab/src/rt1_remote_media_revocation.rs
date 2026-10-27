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
        })
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
        self.worker
            .cancel_confirmed(&self.operation_id, self.generation)
            .map_err(|_| "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED".to_owned())?;
        self.confirmed.store(true, Ordering::Release);
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
