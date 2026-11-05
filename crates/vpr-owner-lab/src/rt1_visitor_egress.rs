//! One publication-scoped admission receipt for a canonical visitor turn.
//! Admission is registered under the SAME workspace transaction as owner pause.
//! Provider I/O must happen only after releasing that transaction, with the
//! canonical runtime turn cancellation handle registered before dispatch.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::{Condvar, Mutex};

#[derive(Default)]
pub(super) struct VisitorEgressState {
    active: Mutex<usize>,
    drained: Condvar,
}

/// This lease spans provider execution AND buffered result handoff.
/// It is NOT a remote audio/video RTP STOP receipt.
pub(crate) struct VisitorEgressPermit {
    state: Arc<VisitorEgressState>,
}

impl VisitorEgressState {
    /// Called only inside the canonical workspace publication transaction.
    /// Never calls an external provider or waits for network I/O.
    pub(super) fn admit(
        self: &Arc<Self>,
        revoked: &AtomicBool,
    ) -> Result<VisitorEgressPermit, String> {
        let mut active = self.active.lock();
        if revoked.load(Ordering::Acquire) {
            return Err("visitor session revoked".into());
        }
        // One canonical turn per visitor: never accumulate unbounded
        // operations behind the engine mutex during a stalled provider.
        if *active != 0 {
            return Err("visitor provider turn already active".into());
        }
        *active = 1;
        Ok(VisitorEgressPermit {
            state: Arc::clone(self),
        })
    }

    /// Local egress completion only; never evidence of remote RTP STOP.
    pub(super) fn quiescent(&self, timeout: Duration) -> bool {
        let mut active = self.active.lock();
        if *active == 0 {
            return true;
        }
        self.drained.wait_for(&mut active, timeout);
        *active == 0
    }
}

impl Drop for VisitorEgressPermit {
    fn drop(&mut self) {
        let mut active = self.state.active.lock();
        debug_assert!(*active > 0);
        *active -= 1;
        if *active == 0 {
            self.state.drained.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revoked_visitor_cannot_admit_provider_egress() {
        let state = Arc::new(VisitorEgressState::default());
        let revoked = AtomicBool::new(true);
        assert!(state.admit(&revoked).is_err());
        assert!(state.quiescent(Duration::ZERO));
    }

    #[test]
    fn admission_is_counted_until_provider_and_result_handoff_finish() {
        let state = Arc::new(VisitorEgressState::default());
        let revoked = AtomicBool::new(false);
        let lease = state.admit(&revoked).unwrap();
        revoked.store(true, Ordering::Release);
        assert!(state.admit(&revoked).is_err());
        assert!(!state.quiescent(Duration::from_millis(1)));
        drop(lease);
        assert!(state.quiescent(Duration::ZERO));
    }

    #[test]
    fn stalled_provider_turn_cannot_queue_a_second_turn() {
        let state = Arc::new(VisitorEgressState::default());
        let revoked = AtomicBool::new(false);
        let first = state.admit(&revoked).unwrap();
        assert_eq!(
            state.admit(&revoked).err().as_deref(),
            Some("visitor provider turn already active")
        );
        assert!(!state.quiescent(Duration::ZERO));
        drop(first);
        assert!(state.quiescent(Duration::ZERO));
        let second = state.admit(&revoked).unwrap();
        drop(second);
    }
}

#[cfg(test)]
mod browser_delivery_race_tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn revoke_during_browser_write_keeps_delivery_non_quiescent_until_writer_exits() {
        let state = Arc::new(VisitorEgressState::default());
        let revoked = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let writer_state = Arc::clone(&state);
        let writer_revoked = Arc::clone(&revoked);
        let writer = std::thread::spawn(move || {
            let _delivery = writer_state.admit(&writer_revoked).unwrap();
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        });
        entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        // Owner pause may commit, but cannot claim a drained delivery while
        // a browser socket is still writing. New provider/HTTP output fails.
        revoked.store(true, Ordering::Release);
        assert!(state.admit(&revoked).is_err());
        assert!(!state.quiescent(Duration::from_millis(5)));
        release_tx.send(()).unwrap();
        writer.join().unwrap();
        assert!(state.quiescent(Duration::ZERO));
        assert!(state.admit(&revoked).is_err());
    }
}
