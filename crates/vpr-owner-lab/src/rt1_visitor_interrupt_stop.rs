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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_revoke_during_interruption_replays_stop_after_shared_fence_release() {
        let interrupt = LiveVisitorInterrupt::default();
        interrupt.stop_in_flight.store(true, Ordering::Release);
        // Owner callback marks the canonical authority revoked, but it cannot
        // issue a competing STOP while the interruption owns the fence.
        interrupt.stop();
        assert!(interrupt.cancelled.load(Ordering::Acquire));
        assert!(interrupt.stop_in_flight.load(Ordering::Acquire));
        assert!(!interrupt.stop_confirmed.load(Ordering::Acquire));

        assert!(release_interruption_stop(&interrupt));
        // No handle was installed in this deterministic admission race.
        // It must remain retryable for a late CREATE rather than confirmed.
        assert!(!interrupt.stop_in_flight.load(Ordering::Acquire));
        assert!(!interrupt.stop_confirmed.load(Ordering::Acquire));
    }

    #[test]
    fn normal_interruption_never_claims_remote_stop_receipt() {
        let interrupt = LiveVisitorInterrupt::default();
        interrupt.stop_in_flight.store(true, Ordering::Release);
        assert!(!release_interruption_stop(&interrupt));
        assert!(!interrupt.stop_in_flight.load(Ordering::Acquire));
        assert!(!interrupt.stop_confirmed.load(Ordering::Acquire));
        assert!(!interrupt.cancelled.load(Ordering::Acquire));
    }
}

/// The engine-backed close/Drop path shares the exact STOP ownership used
/// by asynchronous revoke and synchronous visitor interruption. A preflight
/// atomic load is insufficient: revoke could acquire the lease after it.
/// Local success is NOT a durable remote audio/video STOP receipt.
pub(super) fn stop_engine_serialized(
    interrupt: &LiveVisitorInterrupt,
    engine: &parking_lot::Mutex<crate::state::OwnerLabEngine>,
) -> Result<(), String> {
    if interrupt.stop_in_flight.swap(true, Ordering::AcqRel) {
        return Err("RT1_VISITOR_INTERRUPT_STOP_UNCONFIRMED".into());
    }
    let result = super::stop_if_uncontended(engine);
    resume_revoked_after_engine_stop(interrupt);
    result
}

/// A revoke can lose the STOP lease to close/Drop even when the local engine
/// cleanup succeeds. Local success is not an authenticated remote RTP receipt:
/// replay the pending revoke through the existing canonical callback path.
fn resume_revoked_after_engine_stop(interrupt: &LiveVisitorInterrupt) {
    interrupt.stop_in_flight.store(false, Ordering::Release);
    if interrupt.cancelled.load(Ordering::Acquire) {
        dispatch(interrupt);
    }
}

#[cfg(test)]
mod engine_stop_revoke_race_tests {
    use super::*;

    #[test]
    fn revoked_engine_stop_releases_lease_without_fabricating_remote_receipt() {
        let interrupt = LiveVisitorInterrupt::default();
        interrupt.stop_in_flight.store(true, Ordering::Release);
        interrupt.cancelled.store(true, Ordering::Release);
        // Models a successful local engine cleanup with no remote sender
        // handle yet (e.g. CREATE returned an ambiguous/late session ID).
        resume_revoked_after_engine_stop(&interrupt);
        assert!(!interrupt.stop_in_flight.load(Ordering::Acquire));
        assert!(!interrupt.stop_confirmed.load(Ordering::Acquire));
        assert!(interrupt.cancelled.load(Ordering::Acquire));
    }

    #[test]
    fn ordinary_engine_close_does_not_initiate_remote_revoke() {
        let interrupt = LiveVisitorInterrupt::default();
        interrupt.stop_in_flight.store(true, Ordering::Release);
        resume_revoked_after_engine_stop(&interrupt);
        assert!(!interrupt.stop_in_flight.load(Ordering::Acquire));
        assert!(!interrupt.stop_confirmed.load(Ordering::Acquire));
        assert!(!interrupt.cancelled.load(Ordering::Acquire));
    }
}
