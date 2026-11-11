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

/// Synchronous interruption competes with asynchronous revoke for the SAME
/// provider STOP capability. An in-progress STOP is not a receipt.
/// No second execution path or provider authority is created here.
pub(super) fn stop_for_interruption(
    interrupt: &LiveVisitorInterrupt,
    stop: &vpr_runtime::RealtimeAvatarStopHandle,
) -> Result<(), ()> {
    if interrupt.stop_in_flight.swap(true, Ordering::AcqRel) {
        return Err(());
    }
    let result = stop.stop().map_err(|_| ());
    // An owner revoke may have arrived while interruption owned this fence.
    // The revoke callback was then correctly unable to dispatch another STOP.
    // On release, replay its pending cleanup through the SAME STOP path.
    // A normal voice interruption is NOT a confirmed remote media shutdown.
    let was_revoked = release_interruption_stop(interrupt);
    if was_revoked { Err(()) } else { result }
}

/// Release interruption ownership and resume a revoke that raced with it.
/// No remote STOP receipt is fabricated from a successful voice interruption.
/// A later revoke will acquire the same gate if it arrives after this check.
fn release_interruption_stop(interrupt: &LiveVisitorInterrupt) -> bool {
    interrupt.stop_in_flight.store(false, Ordering::Release);
    if interrupt.cancelled.load(Ordering::Acquire) {
        dispatch(interrupt);
        true
    } else {
        false
    }
}
