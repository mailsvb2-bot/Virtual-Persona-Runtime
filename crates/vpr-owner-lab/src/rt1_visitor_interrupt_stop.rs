//! Retryable, deduplicated RT1 provider STOP dispatch for a revoked visitor.
//! The owner authority fence runs synchronously before entering this helper.
use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::LiveVisitorInterrupt;

/// Dispatch a bounded best-effort STOP without consuming the canonical handle.
/// The explicit close path still requires independently confirmed provider STOP.
pub(super) fn dispatch(interrupt: &LiveVisitorInterrupt) {
    if interrupt.stop_confirmed.load(Ordering::Acquire)
        || interrupt.stop_in_flight.swap(true, Ordering::AcqRel)
    {
        return;
    }
    let stop = interrupt.backend_stop.lock().clone();
    if let Some(stop) = stop {
        let in_flight = Arc::clone(&interrupt.stop_in_flight);
        let confirmed = Arc::clone(&interrupt.stop_confirmed);
        std::thread::spawn(move || {
            match stop.stop() {
                Ok(()) => confirmed.store(true, Ordering::Release),
                Err(error) => eprintln!("RT1 best-effort backend STOP failed: {error:?}"),
            }
            in_flight.store(false, Ordering::Release);
        });
    } else {
        // A CREATE may still be pending. Missing handle must not poison retries.
        interrupt.stop_in_flight.store(false, Ordering::Release);
    }
}
