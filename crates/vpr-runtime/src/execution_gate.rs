use std::sync::{Arc, Condvar, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;
use vpr_integration::CancellationProbe;

use crate::cancellation::ProviderCancellation;

/// One canonical per-session serialization boundary for owner and visitor
/// provider operations. Never hold the authority lock across network I/O.
#[derive(Debug, Clone, Default)]
pub(crate) struct SessionExecutionGate {
    inner: Arc<RwLock<()>>,
    pending: Arc<PendingProviderEgress>,
}

#[derive(Debug, Default)]
struct PendingProviderEgress {
    active: Mutex<usize>,
    drained: Condvar,
}

/// A tracked provider operation, admitted before the owner can commit revoke.
/// Dropping this guard acknowledges local transport completion, NOT remote
/// audio/video STOP.
#[derive(Debug)]
pub(crate) struct ProviderEgressGuard {
    pending: Arc<PendingProviderEgress>,
}

impl Drop for ProviderEgressGuard {
    fn drop(&mut self) {
        let Ok(mut active) = self.pending.active.lock() else {
            return;
        };
        *active -= 1;
        if *active == 0 {
            self.pending.drained.notify_all();
        }
    }
}

impl SessionExecutionGate {
    pub(crate) fn read(&self) -> Result<RwLockReadGuard<'_, ()>, ()> {
        self.inner.read().map_err(|_| ())
    }

    pub(crate) fn write(&self) -> Result<RwLockWriteGuard<'_, ()>, ()> {
        self.inner.write().map_err(|_| ())
    }

    /// Linearize dispatch admission with authority revoke, but release the
    /// lock before calling a potentially stalled external provider. Revoke
    /// cancels already-admitted operations; callers can separately require
    /// bounded quiescence before treating local egress as stopped.
    pub(crate) fn begin_provider_egress(
        &self,
        cancellation: &ProviderCancellation,
    ) -> Result<ProviderEgressGuard, ()> {
        let _admission = self.read()?;
        if cancellation.is_cancelled() {
            return Err(());
        }
        let mut active = self.pending.active.lock().map_err(|_| ())?;
        *active = active.checked_add(1).ok_or(())?;
        drop(active);
        Ok(ProviderEgressGuard {
            pending: Arc::clone(&self.pending),
        })
    }

    /// A local transport drain receipt, not a remote RTP STOP receipt.
    /// Bounded waiting never holds the publication/authorization write lock.
    pub(crate) fn await_provider_quiescence(&self, timeout: Duration) -> bool {
        let Ok(active) = self.pending.active.lock() else {
            return false;
        };
        self.pending
            .drained
            .wait_timeout_while(active, timeout, |count| *count != 0)
            .is_ok_and(|(count, _)| *count == 0)
    }
}
