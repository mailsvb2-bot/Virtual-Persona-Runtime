use super::*;

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
fn abandoned_terminal_stream_is_evicted_before_next_request() {
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
    assert!(registry.wait_events(1).is_none());
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
