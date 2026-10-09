use super::*;
use vpr_owner_lab::LabVoiceSegment;

fn segment(sequence: u64) -> VoiceStreamEvent {
    VoiceStreamEvent::Segment {
        segment: LabVoiceSegment {
            evidence_turn_sequence: sequence,
            evidence_output_sequence: 1,
            client_command: None,
        },
    }
}

#[test]
fn terminal_reply_survives_next_request_and_cannot_be_silently_evicted() {
    let registry = VoiceStreamRegistry::default();
    assert!(registry.begin(1));
    registry.finish(
        1,
        VoiceStreamEvent::Failed {
            code: "PROVIDER_TIMEOUT".into(),
            diagnostic: None,
        },
    );

    assert!(registry.begin(2));
    let old = registry
        .wait_events(1)
        .expect("old terminal must remain readable");
    assert!(old.terminal);
    assert_eq!(old.events.len(), 1);
    assert!(!registry.begin(3));

    registry.finish(
        2,
        VoiceStreamEvent::Failed {
            code: "TURN_CANCELLED".into(),
            diagnostic: None,
        },
    );
    let terminal = registry
        .wait_events(2)
        .expect("terminal stream must remain pollable");
    assert!(terminal.terminal);
    assert_eq!(terminal.events.len(), 1);
}

#[test]
fn pending_stream_is_never_evicted_and_event_queue_is_bounded() {
    let registry = VoiceStreamRegistry::default();
    assert!(registry.begin(10));
    assert!(!registry.begin(11));

    for index in 0..MAX_PENDING_VOICE_STREAM_EVENTS.saturating_sub(1) {
        registry
            .push(10, segment(u64::try_from(index + 1).unwrap()))
            .unwrap();
    }
    assert!(matches!(
        registry.push(10, segment(999)),
        Err(LabError::InvalidState)
    ));

    registry.finish(
        10,
        VoiceStreamEvent::Failed {
            code: "INVALID_STATE_TRANSITION".into(),
            diagnostic: None,
        },
    );
    let response = registry
        .wait_events(10)
        .expect("stream must still be available");
    assert!(response.terminal);
    assert_eq!(response.events.len(), MAX_PENDING_VOICE_STREAM_EVENTS);
    assert!(registry.wait_events(10).is_none());
}

#[test]
fn quiescence_does_not_require_terminal_stream_polling() {
    let registry = VoiceStreamRegistry::default();
    assert!(registry.begin(1));
    registry.finish(
        1,
        VoiceStreamEvent::Failed {
            code: "PROVIDER_TIMEOUT".into(),
            diagnostic: None,
        },
    );
    assert!(registry.wait_until_quiescent());
}

#[test]
fn terminal_handoff_releases_voice_gate_only_once_even_after_next_turn_starts() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let registry = VoiceStreamRegistry::default();
    let busy = AtomicBool::new(true);
    let cancelled = AtomicBool::new(true);
    let mut old_guard = http_evidence::VoiceBusyGuard::new(&busy, &cancelled);
    assert!(registry.begin(1));

    // The previous stream must become terminal while the registry is locked,
    // and the same owner releases busy before any consumer is notified.
    registry.finish_with_unlock(
        1,
        VoiceStreamEvent::Failed {
            code: "TURN_CANCELLED".into(),
            diagnostic: None,
        },
        || {
            assert!(busy.load(Ordering::Acquire));
            old_guard.release();
        },
    );
    assert!(!busy.load(Ordering::Acquire));
    assert!(!cancelled.load(Ordering::Acquire));

    // A new request may now own the same atomic bit; dropping the OLD guard
    // must not unlock or reset cancellation for the NEW owner.
    assert!(
        busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    );
    cancelled.store(true, Ordering::Release);
    drop(old_guard);
    assert!(busy.load(Ordering::Acquire));
    assert!(cancelled.load(Ordering::Acquire));

    let terminal = registry
        .wait_events(1)
        .expect("old terminal remains consumable");
    assert!(terminal.terminal);
    assert_eq!(terminal.events.len(), 1);
    assert!(
        registry.begin(2),
        "new stream may start after atomic handoff"
    );
}

#[test]
fn two_unconsumed_terminal_events_apply_backpressure_without_evidence_loss() {
    let registry = VoiceStreamRegistry::default();
    assert!(registry.begin(1));
    registry.finish(
        1,
        VoiceStreamEvent::Failed {
            code: "PROVIDER_TIMEOUT".into(),
            diagnostic: None,
        },
    );
    assert!(registry.begin(2));
    registry.finish(
        2,
        VoiceStreamEvent::Failed {
            code: "TURN_CANCELLED".into(),
            diagnostic: None,
        },
    );
    assert!(
        !registry.begin(3),
        "must not discard an unconsumed terminal event"
    );
    assert!(registry.wait_events(1).expect("first reply").terminal);
    assert!(
        registry.begin(3),
        "a consumed terminal slot is immediately reusable"
    );
    assert!(registry.wait_events(2).expect("second reply").terminal);
}
