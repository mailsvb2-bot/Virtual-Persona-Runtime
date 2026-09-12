use vpr_domain::{
    CorrelationId, OutputCheckpoint, OutputDeliveryState, PersonaId, PersonaIdentity, PersonaMode,
    PersonaVersion, Rt0ReasonCode, SessionId, TurnId, TurnState,
};
use vpr_policy::{AuthorityLayer, AuthorityScope, ConsentState, DataClass, EffectiveAuthority};
use vpr_runtime::{ActiveSession, ActiveTurn, RuntimeDenyReason};

#[test]
fn session_revoke_during_stream_blocks_new_egress_and_preserves_spoken_prefix_only() {
    let persona = PersonaIdentity::new(
        PersonaId::new("persona-owner").unwrap(),
        PersonaVersion::new(1).unwrap(),
        PersonaMode::DigitalTwin,
    );
    let mut session = ActiveSession::new(
        SessionId::new("session-owner-test").unwrap(),
        persona.id().clone(),
        Some(10_000),
        true,
        ConsentState::Granted,
        false,
    );
    session.activate().unwrap();

    let provider_scope = AuthorityScope::new("provider.egress").unwrap();
    let authority =
        EffectiveAuthority::compose(&[AuthorityLayer::new([provider_scope.clone()], [])]);
    let mut turn = ActiveTurn::new(
        TurnId::new("turn-owner-test").unwrap(),
        CorrelationId::new("corr-owner-test").unwrap(),
        &persona,
        &session,
    )
    .unwrap();
    turn.authorize(1_000).unwrap();
    turn.begin_processing().unwrap();
    let permit = turn
        .begin_external_provider_call(1_000, &authority, &provider_scope, DataClass::Biometric)
        .unwrap();
    turn.begin_output().unwrap();

    let spoken = turn.begin_output_segment().unwrap();
    turn.mark_output_sent(spoken).unwrap();
    turn.mark_output_played(spoken).unwrap();
    let tail = turn.begin_output_segment().unwrap();
    turn.mark_output_sent(tail).unwrap();

    session.revoke().unwrap();
    assert!(permit.is_cancelled());
    let denied =
        turn.begin_external_provider_call(1_001, &authority, &provider_scope, DataClass::Biometric);
    assert!(matches!(denied, Err(RuntimeDenyReason::AuthorizationStale)));
    assert_eq!(
        denied.unwrap_err().reason_code(),
        Rt0ReasonCode::AuthRevoked
    );

    turn.interrupt().unwrap();
    assert_eq!(turn.state(), TurnState::Cancelled);
    let segments = turn.output_segments();
    assert_eq!(segments.len(), 2);
    assert_eq!(
        segments[0].state(),
        OutputDeliveryState::Cancelled {
            reached: OutputCheckpoint::Played
        }
    );
    assert!(segments[0].eligible_as_spoken());
    assert_eq!(
        segments[1].state(),
        OutputDeliveryState::Cancelled {
            reached: OutputCheckpoint::Sent
        }
    );
    assert!(!segments[1].eligible_as_spoken());
}
