use std::sync::Mutex;

use vpr_domain::{
    CorrelationId, OutputCheckpoint, OutputDeliveryState, PersonaId, PersonaIdentity, PersonaMode,
    PersonaVersion, Rt0ReasonCode, SessionId, TurnId, TurnState,
};
use vpr_integration::{
    CancellationProbe, RealtimeOutputPort, RealtimeTextOutputEvent, TransportError,
    TransportErrorKind,
};
use vpr_policy::{AuthorityLayer, AuthorityScope, ConsentState, EffectiveAuthority};
use vpr_runtime::{ActiveSession, ActiveTurn, OutputDeliveryError, SessionSecurityConfig};

#[derive(Default)]
struct RecordingTransport {
    events: Mutex<Vec<RealtimeTextOutputEvent>>,
}

impl RealtimeOutputPort for RecordingTransport {
    fn send_text(
        &self,
        event: &RealtimeTextOutputEvent,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), TransportError> {
        assert!(!cancellation.is_cancelled());
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }
}

struct FailingTransport(TransportErrorKind);

impl RealtimeOutputPort for FailingTransport {
    fn send_text(
        &self,
        _event: &RealtimeTextOutputEvent,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<(), TransportError> {
        Err(TransportError {
            kind: self.0,
            retryable: self.0 == TransportErrorKind::Unavailable,
        })
    }
}

fn active_turn(name: &str) -> ActiveTurn {
    let persona = PersonaIdentity::new(
        PersonaId::new(format!("persona-{name}")).unwrap(),
        PersonaVersion::new(1).unwrap(),
        PersonaMode::DigitalTwin,
    );
    let scope = AuthorityScope::new("provider.egress").unwrap();
    let authority = EffectiveAuthority::compose(&[AuthorityLayer::new([scope], [])]);
    let mut session = ActiveSession::new(
        SessionId::new(format!("session-{name}")).unwrap(),
        persona.id().clone(),
        SessionSecurityConfig::new(authority, None, true, ConsentState::Granted, false),
    );
    session.activate().unwrap();
    let turn = ActiveTurn::new(
        TurnId::new(format!("turn-{name}")).unwrap(),
        CorrelationId::new(format!("corr-{name}")).unwrap(),
        &persona,
        &session,
    )
    .unwrap();
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();
    turn.begin_output().unwrap();
    turn
}

#[test]
fn successful_transport_is_the_only_path_to_confirmed_sent_and_played() {
    let turn = active_turn("delivery-success");
    let transport = RecordingTransport::default();
    let handle = turn.deliver_text(&transport, "Привет").unwrap();

    let events = transport.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].turn_id.as_str(), "turn-delivery-success");
    assert_eq!(events[0].correlation_id.as_str(), "corr-delivery-success");
    assert_eq!(events[0].sequence, handle.sequence());
    assert_eq!(events[0].text, "Привет");
    drop(events);

    assert_eq!(turn.output_segments()[0].state(), OutputDeliveryState::Sent);
    turn.acknowledge_output_played(&handle).unwrap();
    turn.acknowledge_output_played(&handle).unwrap();
    assert_eq!(
        turn.output_segments()[0].state(),
        OutputDeliveryState::Played
    );
}

#[test]
fn definite_transport_failure_never_fabricates_sent_evidence() {
    let turn = active_turn("delivery-failure");
    let error = turn
        .deliver_text(&FailingTransport(TransportErrorKind::Unavailable), "Привет")
        .unwrap_err();
    assert!(matches!(
        error,
        OutputDeliveryError::Transport(TransportError {
            kind: TransportErrorKind::Unavailable,
            ..
        })
    ));
    assert_eq!(
        turn.output_segments()[0].state(),
        OutputDeliveryState::Generated
    );
    assert!(!turn.output_segments()[0].eligible_as_spoken());
}

#[test]
fn uncertain_send_is_explicit_and_reconciles_without_blind_retry() {
    let turn = active_turn("delivery-uncertain");
    let error = turn
        .deliver_text(
            &FailingTransport(TransportErrorKind::DeliveryUncertain),
            "Привет",
        )
        .unwrap_err();
    assert_eq!(
        turn.output_segments()[0].state(),
        OutputDeliveryState::DeliveryUncertain
    );
    let handle = error
        .uncertain_handle()
        .expect("uncertain transport must return reconciliation handle");

    turn.acknowledge_output_sent(handle).unwrap();
    turn.acknowledge_output_sent(handle).unwrap();
    assert_eq!(turn.output_segments()[0].state(), OutputDeliveryState::Sent);
    turn.acknowledge_output_played(handle).unwrap();
    assert_eq!(
        turn.output_segments()[0].state(),
        OutputDeliveryState::Played
    );
}

#[test]
fn late_playback_resolves_cancelled_uncertain_delivery_without_reanimating_turn() {
    let turn = active_turn("delivery-late-playback");
    let error = turn
        .deliver_text(
            &FailingTransport(TransportErrorKind::DeliveryUncertain),
            "Привет",
        )
        .unwrap_err();
    let handle = error.uncertain_handle().unwrap();

    turn.interrupt().unwrap();
    assert_eq!(turn.state(), TurnState::Cancelled);
    assert_eq!(
        turn.output_segments()[0].state(),
        OutputDeliveryState::CancelledDeliveryUncertain
    );

    turn.acknowledge_output_played(handle).unwrap();
    turn.acknowledge_output_played(handle).unwrap();
    assert_eq!(turn.state(), TurnState::Cancelled);
    assert_eq!(
        turn.output_segments()[0].state(),
        OutputDeliveryState::Cancelled {
            reached: OutputCheckpoint::Played
        }
    );
    assert!(turn.output_segments()[0].eligible_as_spoken());
}

#[test]
fn delivery_handle_cannot_advance_another_turn() {
    let source = active_turn("delivery-source");
    let target = active_turn("delivery-target");
    let handle = source
        .deliver_text(&RecordingTransport::default(), "Привет")
        .unwrap();
    assert_eq!(
        target.acknowledge_output_played(&handle),
        Err(Rt0ReasonCode::InvalidStateTransition)
    );
}
