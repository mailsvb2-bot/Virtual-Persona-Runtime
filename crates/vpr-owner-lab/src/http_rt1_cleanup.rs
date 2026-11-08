//! Bounded reconciliation of uncertain RT1 worker STOP results.
//! This is part of the canonical owner-lab HTTP state, never a second runtime.
use std::sync::Arc;

use super::{AppState, Rt1HttpState, Rt1RemoteMediaRevocation};

const MAX_TRACKED_REMOTE_OPERATIONS: usize = 64;

impl Rt1HttpState {
    pub(super) fn register_remote(
        &self,
        retry_id: String,
        remote: Arc<Rt1RemoteMediaRevocation>,
    ) -> Result<(), String> {
        let mut pending = self.remote_cleanup.lock();
        if pending.len() >= MAX_TRACKED_REMOTE_OPERATIONS || pending.contains_key(&retry_id) {
            return Err("RT1_REMOTE_CLEANUP_CAPACITY_EXCEEDED".into());
        }
        // The callback only signals this already-running reconciler; it
        // never starts a second STOP worker or makes a network request.
        let wake = Arc::clone(&self.cleanup_notify);
        remote.install_revoke_notifier(move || {
            let (sequence, ready) = &*wake;
            let mut number = sequence.lock();
            *number = number.wrapping_add(1);
            drop(number);
            ready.notify_one();
        });
        pending.insert(retry_id, remote);
        Ok(())
    }

    pub(super) fn reconcile_remote_stops(&self) {
        // Only revoked operations are eligible. Never stop an active
        // visitor merely because its handle is present in this registry.
        let pending: Vec<_> = self
            .remote_cleanup
            .lock()
            .iter()
            .filter(|(_, remote)| remote.is_revoked())
            .map(|(id, remote)| (id.clone(), Arc::clone(remote)))
            .collect();
        for (id, remote) in pending {
            if remote.is_stop_confirmed() || remote.revoke_confirmed().is_ok() {
                let mut registry = self.remote_cleanup.lock();
                if registry
                    .get(&id)
                    .is_some_and(|current| Arc::ptr_eq(current, &remote))
                {
                    registry.remove(&id);
                }
            }
        }
    }
}

/// A single bounded cleanup worker for the lifetime of the local owner lab.
/// Unknown STOP outcomes retain their retry handles and cannot claim success.
pub(crate) fn spawn_cleanup_reconciler(state: &Arc<AppState>) {
    let wake = Arc::clone(&state.rt1.cleanup_notify);
    let state = Arc::downgrade(state);
    std::thread::spawn(move || loop {
        // Capture the generation before reconciling so a concurrent revoke
        // cannot be lost between the STOP attempt and the bounded wait.
        let (sequence, ready) = &*wake;
        let before = *sequence.lock();
        let Some(state) = state.upgrade() else {
            break;
        };
        state.rt1.reconcile_remote_stops();
        let mut observed = sequence.lock();
        if *observed == before {
            ready.wait_for(&mut observed, std::time::Duration::from_secs(2));
        }
    });
}
