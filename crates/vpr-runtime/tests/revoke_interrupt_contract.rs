use vpr_domain::{
    CorrelationId, OutputCheckpoint, OutputDeliveryState, PersonaId, PersonaIdentity, PersonaMode,
    PersonaVersion, RealtimeSession, RealtimeSessionState, Rt0ReasonCode, SessionId, TurnId,
    TurnState,
};
use vpr_policy::{
    AuthorityLayer, AuthorityScope, AuthorizationState, EffectiveAuthority, EgressDecision,
    EgressReason,
};
use vpr_runtime::{ActiveTurn, RuntimeDenyReason, authorize_external_provider_call};

#[test]
fn revoke_during_active_turn_blocks_next_egress_and_interrupts_unplayed_tail() {
    let persona = PersonaIdentity {
        id: PersonaId::new("persona-owner").unwrap(),
        version: PersonaVersion::new(1).unwrap(),
        mode: PersonaMode::DigitalTwin,
    };
    let mut session = RealtimeSession::new(
        SessionId::new("session-owner-test").unwrap(),
        persona.id.clone(),
    );
    session.transition(RealtimeSessionState::Active).unwrap();

    let provider_scope = AuthorityScope::new("provider.egress").unwrap();
    let authority =
        EffectiveAuthority::compose(&[AuthorityLayer::new([provider_scope.clone()], [])]);
    let mut authorization = AuthorizationState::new(Some(10_000));
    let cached = authorization.snapshot();

    let mut turn = ActiveTurn::new(
        TurnId::new("turn-owner-test").unwrap(),
        CorrelationId::new("corr-owner-test").unwrap(),
        &persona,
        cached,
    );
    turn.transition(TurnState::Authorized).unwrap();
    turn.transition(TurnState::Processing).unwrap();

    authorize_external_provider_call(
        authorization,
        cached,
        1_000,
        &authority,
        &provider_scope,
        EgressDecision::Allow(EgressReason::Allowed),
    )
    .unwrap();

    turn.transition(TurnState::Outputting).unwrap();
    turn.mark_output_generated().unwrap();
    turn.mark_output_sent().unwrap();

    session.transition(RealtimeSessionState::Revoked).unwrap();
    authorization.revoke().unwrap();

    let denied = authorize_external_provider_call(
        authorization,
        cached,
        1_001,
        &authority,
        &provider_scope,
        EgressDecision::Allow(EgressReason::Allowed),
    );
    assert_eq!(denied, Err(RuntimeDenyReason::AuthorizationStale));
    assert_eq!(
        denied.unwrap_err().reason_code(),
        Rt0ReasonCode::AuthRevoked
    );

    turn.interrupt().unwrap();
    assert_eq!(turn.state(), TurnState::Cancelled);
    assert_eq!(
        turn.output().state(),
        OutputDeliveryState::Cancelled {
            reached: OutputCheckpoint::Sent
        }
    );
    assert!(!turn.output().eligible_as_spoken());
    assert_eq!(
        turn.snapshot().authorization_epoch,
        cached.epoch,
        "execution evidence stays bound to the authority revision actually used"
    );
}
