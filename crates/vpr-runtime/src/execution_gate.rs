use std::collections::HashMap;
use std::sync::{
    Arc, Condvar, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use crate::cancellable_tcp_writer::TcpWriteRevocation;
use vpr_integration::CancellationProbe;

use crate::cancellation::ProviderCancellation;

/// One canonical per-session serialization boundary for owner and visitor
/// provider operations. Never hold the authority lock across network I/O.
#[derive(Debug, Clone, Default)]
pub(crate) struct SessionExecutionGate {
    inner: Arc<RwLock<()>>,
    pending: Arc<PendingProviderEgress>,
    tcp_responses: Arc<PendingTcpResponses>,
}

#[derive(Debug)]
struct PendingTcpResponses {
    next_id: AtomicU64,
    writers: Mutex<HashMap<u64, TcpWriteRevocation>>,
}

impl Default for PendingTcpResponses {
    fn default() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            writers: Mutex::new(HashMap::new()),
        }
    }
}

/// Registration for one real HTTP response socket. Dropping it unregisters
/// the completed response; authority revoke shuts down every still-registered
/// socket through the same session execution gate as provider egress.
#[derive(Debug)]
pub struct TcpResponseRegistration {
    pending: Arc<PendingTcpResponses>,
    id: u64,
}

impl Drop for TcpResponseRegistration {
    fn drop(&mut self) {
        if let Ok(mut writers) = self.pending.writers.lock() {
            writers.remove(&self.id);
        }
    }
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

    /// Register an actual client TCP response socket while holding the same
    /// admission read lock used by provider dispatch. The caller must first
    /// verify the session authority; a concurrent revoke cannot miss this
    /// registration because it needs the matching write lock.
    pub(crate) fn register_tcp_response_writer_locked(
        &self,
        writer: TcpWriteRevocation,
    ) -> Result<TcpResponseRegistration, ()> {
        let id = self
            .tcp_responses
            .next_id
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |next| next.checked_add(1))
            .map_err(|_| ())?;
        let mut writers = self.tcp_responses.writers.lock().map_err(|_| ())?;
        writers.insert(id, writer);
        Ok(TcpResponseRegistration {
            pending: Arc::clone(&self.tcp_responses),
            id,
        })
    }

    /// Must be called while the session execution write lock is held, after
    /// authority/policy mutation. Socket shutdown is local and bounded; it is
    /// not a remote audio/video sender STOP receipt.
    pub(crate) fn revoke_tcp_response_writers_locked(&self) -> Result<(), ()> {
        let writers = self
            .tcp_responses
            .writers
            .lock()
            .map_err(|_| ())?
            .drain()
            .map(|(_, writer)| writer)
            .collect::<Vec<_>>();
        for writer in writers {
            writer.revoke();
        }
        Ok(())
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
