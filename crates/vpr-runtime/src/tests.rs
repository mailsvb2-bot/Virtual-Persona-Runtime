use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc,
};
use std::thread;
use std::time::Duration;

use crate::cancellation::TurnCancellation;
use crate::clock::RuntimeClock;
use crate::provider::ProviderOperation;
use vpr_domain::{
    CorrelationId, OutputCheckpoint, OutputDeliveryState, PersonaId, PersonaIdentity, PersonaMode,
    PersonaVersion, RealtimeSessionState, Rt0ReasonCode, SessionId, TurnId, TurnState,
};
use vpr_integration::{CancellationProbe, ProviderErrorKind};
use vpr_policy::{AuthorityLayer, AuthorityScope, ConsentState, DataClass, EffectiveAuthority};

#[derive(Debug)]
struct ManualClock(AtomicU64);

impl ManualClock {
    fn new(now_millis: u64) -> Self {
        Self(AtomicU64::new(now_millis))
    }

    fn set(&self, now_millis: u64) {
        self.0.store(now_millis, Ordering::SeqCst);
    }
}

impl RuntimeClock for ManualClock {
    fn now_millis(&self) -> Option<u64> {
        Some(self.0.load(Ordering::SeqCst))
    }
}

fn persona() -> PersonaIdentity {
    PersonaIdentity::new(
        PersonaId::new("persona-1").unwrap(),
        PersonaVersion::new(1).unwrap(),
        PersonaMode::DigitalTwin,
    )
}

fn active_session() -> ActiveSession {
    let identity = persona();
    let (_, authority) = allowed_provider_authority();
    let mut session = ActiveSession::new(
        SessionId::new("session-1").unwrap(),
        identity.id().clone(),
        SessionSecurityConfig::new(authority, None, true, ConsentState::Granted, false),
    );
    session.activate().unwrap();
    session
}

fn allowed_provider_authority() -> (AuthorityScope, EffectiveAuthority) {
    let scope = AuthorityScope::new("provider.egress").unwrap();
    let authority = EffectiveAuthority::compose(&[AuthorityLayer::new([scope.clone()], [])]);
    (scope, authority)
}

fn turn(session: &ActiveSession, name: &str) -> ActiveTurn {
    ActiveTurn::new(
        TurnId::new(format!("turn-{name}")).unwrap(),
        CorrelationId::new(format!("corr-{name}")).unwrap(),
        &persona(),
        session,
    )
    .unwrap()
}

#[test]
fn cancellation_propagates_to_clones() {
    let cancellation = TurnCancellation::default();
    let provider_view = cancellation.clone();
    assert!(!CancellationProbe::is_cancelled(&provider_view));
    cancellation.cancel();
    assert!(CancellationProbe::is_cancelled(&provider_view));
}

#[test]
fn turn_requires_active_matching_session() {
    let identity = persona();
    let (_, authority) = allowed_provider_authority();
    let session = ActiveSession::new(
        SessionId::new("created").unwrap(),
        identity.id().clone(),
        SessionSecurityConfig::new(authority, None, true, ConsentState::Granted, false),
    );
    assert!(matches!(
        ActiveTurn::new(
            TurnId::new("turn-created").unwrap(),
            CorrelationId::new("corr-created").unwrap(),
            &identity,
            &session,
        ),
        Err(RuntimeDenyReason::InvalidTurnState)
    ));
}

#[test]
fn provider_call_before_turn_authorization_is_rejected() {
    let (scope, _authority) = allowed_provider_authority();
    let session = active_session();
    let turn = turn(&session, "premature");
    assert!(matches!(
        turn.issue_provider_permit(&scope, DataClass::Public),
        Err(RuntimeDenyReason::InvalidTurnState)
    ));
}

#[test]
fn session_revoke_cancels_existing_permit_and_denies_new_work() {
    let (scope, _authority) = allowed_provider_authority();
    let mut session = active_session();
    let turn = turn(&session, "session-revoke");
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();
    let permit = turn
        .issue_provider_permit(&scope, DataClass::Public)
        .unwrap();
    assert!(!permit.cancellation.is_cancelled());

    session.revoke().unwrap();
    assert_eq!(session.state(), RealtimeSessionState::Revoked);
    assert!(permit.cancellation.is_cancelled());
    assert!(matches!(
        turn.issue_provider_permit(&scope, DataClass::Public),
        Err(RuntimeDenyReason::AuthorizationStale)
    ));
}

#[test]
fn local_only_current_policy_blocks_external_provider() {
    let (scope, _authority) = allowed_provider_authority();
    let session = active_session();
    session.set_local_only_required(true).unwrap();
    let turn = turn(&session, "local-only");
    turn.authorize().unwrap();
    assert!(matches!(
        turn.issue_provider_permit(&scope, DataClass::Public),
        Err(RuntimeDenyReason::LocalOnlyRequired)
    ));
}

#[test]
fn policy_change_cancels_existing_permit_and_stales_bound_turn() {
    let (scope, _authority) = allowed_provider_authority();
    let session = active_session();
    let turn = turn(&session, "policy-change");
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();
    let permit = turn
        .issue_provider_permit(&scope, DataClass::Biometric)
        .unwrap();
    assert!(!permit.cancellation.is_cancelled());

    session.set_consent(ConsentState::Revoked).unwrap();
    assert!(permit.cancellation.is_cancelled());
    assert!(matches!(
        turn.issue_provider_permit(&scope, DataClass::Biometric),
        Err(RuntimeDenyReason::EgressPolicyStale)
    ));
}

#[test]
fn current_missing_biometric_consent_uses_stable_reason_code() {
    let (scope, _authority) = allowed_provider_authority();
    let session = active_session();
    session.set_consent(ConsentState::Missing).unwrap();
    let turn = turn(&session, "consent");
    turn.authorize().unwrap();
    let denied = turn.issue_provider_permit(&scope, DataClass::Biometric);
    assert!(matches!(denied, Err(RuntimeDenyReason::ConsentRequired)));
    assert_eq!(
        denied.unwrap_err().reason_code(),
        Rt0ReasonCode::ConsentRequired
    );
}

#[test]
fn authorization_replacement_cancels_permit_and_stales_turn() {
    let (scope, _authority) = allowed_provider_authority();
    let session = active_session();
    let turn = turn(&session, "auth-replace");
    turn.authorize().unwrap();
    let permit = turn
        .issue_provider_permit(&scope, DataClass::Public)
        .unwrap();
    session.refresh_authorization(None).unwrap();
    assert!(permit.cancellation.is_cancelled());
    assert!(matches!(
        turn.issue_provider_permit(&scope, DataClass::Public),
        Err(RuntimeDenyReason::AuthorizationStale)
    ));
}

#[test]
fn execution_snapshot_revisions_gate_provider_execution() {
    let (scope, _authority) = allowed_provider_authority();
    let session = active_session();
    let turn = turn(&session, "snapshot");
    turn.authorize().unwrap();
    assert!(
        turn.issue_provider_permit(&scope, DataClass::Public)
            .is_ok()
    );
    session.refresh_authorization(None).unwrap();
    assert!(matches!(
        turn.issue_provider_permit(&scope, DataClass::Public),
        Err(RuntimeDenyReason::AuthorizationStale)
    ));
    assert_eq!(turn.snapshot().authorization_epoch().get(), 1);
    assert_eq!(turn.snapshot().egress_policy_revision().get(), 1);
}

#[test]
fn interruption_preserves_played_prefix_and_unplayed_tail_per_segment() {
    let session = active_session();
    let turn = turn(&session, "stream");
    let provider_view = turn.cancellation.clone();
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();
    turn.begin_output().unwrap();
    let spoken = turn.begin_output_segment().unwrap();
    turn.mark_output_sent(spoken).unwrap();
    turn.mark_output_played(spoken).unwrap();
    let tail = turn.begin_output_segment().unwrap();
    turn.mark_output_sent(tail).unwrap();
    turn.interrupt().unwrap();
    assert_eq!(turn.state(), TurnState::Cancelled);
    assert!(provider_view.is_cancelled());
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

#[test]
fn deny_and_fail_are_real_terminal_paths() {
    let session = active_session();
    let denied = turn(&session, "denied");
    denied.authorize().unwrap();
    denied.begin_processing().unwrap();
    denied.deny().unwrap();
    assert_eq!(denied.state(), TurnState::Denied);
    assert!(denied.begin_processing().is_err());

    let failed = turn(&session, "failed");
    failed.authorize().unwrap();
    failed.begin_processing().unwrap();
    failed.begin_output().unwrap();
    let segment = failed.begin_output_segment().unwrap();
    failed.mark_output_sent(segment).unwrap();
    failed.fail().unwrap();
    assert_eq!(failed.state(), TurnState::Failed);
    assert!(matches!(
        failed.output_segments()[0].state(),
        OutputDeliveryState::Cancelled {
            reached: OutputCheckpoint::Sent
        }
    ));
}

#[test]
fn denied_turn_cannot_emit_output_evidence() {
    let session = active_session();
    let turn = turn(&session, "denied-output");
    turn.deny().unwrap();
    assert_eq!(
        turn.begin_output_segment(),
        Err(Rt0ReasonCode::InvalidStateTransition)
    );
    assert!(turn.output_segments().is_empty());
}

#[test]
fn invalid_interrupt_does_not_partially_mutate_output_evidence() {
    let session = active_session();
    let turn = turn(&session, "terminal");
    turn.deny().unwrap();
    assert_eq!(turn.interrupt(), Err(Rt0ReasonCode::InvalidStateTransition));
    assert!(turn.output_segments().is_empty());
    assert!(!turn.cancellation.is_cancelled());
}

#[test]
fn authority_narrowing_invalidates_old_turn_and_denies_new_scope() {
    let (scope, _) = allowed_provider_authority();
    let session = active_session();
    let old_turn = turn(&session, "authority-narrow-old");
    old_turn.authorize().unwrap();
    old_turn.begin_processing().unwrap();
    let permit = old_turn
        .issue_provider_permit(&scope, DataClass::Public)
        .unwrap();

    session
        .replace_authority(EffectiveAuthority::default(), None)
        .unwrap();
    assert!(permit.cancellation.is_cancelled());
    assert!(matches!(
        old_turn.issue_provider_permit(&scope, DataClass::Public),
        Err(RuntimeDenyReason::AuthorizationStale)
    ));

    let fresh_turn = turn(&session, "authority-narrow-new");
    fresh_turn.authorize().unwrap();
    assert!(matches!(
        fresh_turn.issue_provider_permit(&scope, DataClass::Public),
        Err(RuntimeDenyReason::AuthorityDenied)
    ));
}

#[test]
fn provider_operation_classification_is_runtime_owned() {
    assert_eq!(ProviderOperation::Llm.data_class(), DataClass::Biometric);
    assert_eq!(ProviderOperation::Stt.data_class(), DataClass::Biometric);
    assert_eq!(ProviderOperation::Tts.data_class(), DataClass::Biometric);
    assert_eq!(ProviderOperation::Avatar.data_class(), DataClass::Biometric);
}

#[test]
fn revocation_blocks_late_output_mutations_and_completion() {
    let (scope, _) = allowed_provider_authority();
    let mut session = active_session();
    let turn = turn(&session, "late-output");
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();
    turn.begin_output().unwrap();
    let segment = turn.begin_output_segment().unwrap();
    turn.mark_output_sent(segment).unwrap();

    session.revoke().unwrap();
    assert_eq!(
        turn.begin_output_segment(),
        Err(Rt0ReasonCode::TurnCancelled)
    );
    assert_eq!(
        turn.mark_output_played(segment),
        Err(Rt0ReasonCode::TurnCancelled)
    );
    assert_eq!(turn.complete(), Err(Rt0ReasonCode::TurnCancelled));
    assert!(matches!(
        turn.issue_provider_permit(&scope, DataClass::Public),
        Err(RuntimeDenyReason::AuthorizationStale)
    ));

    turn.interrupt().unwrap();
    assert_eq!(turn.state(), TurnState::Cancelled);
}

#[test]
fn retained_operation_cannot_bypass_expired_lease() {
    let identity = persona();
    let clock = Arc::new(ManualClock::new(100));
    let (_, authority) = allowed_provider_authority();
    let mut session = ActiveSession::with_clock(
        SessionId::new("session-clock").unwrap(),
        identity.id().clone(),
        SessionSecurityConfig::new(authority, Some(200), true, ConsentState::Granted, false),
        clock.clone(),
    );
    session.activate().unwrap();
    let turn = ActiveTurn::new(
        TurnId::new("turn-clock").unwrap(),
        CorrelationId::new("corr-clock").unwrap(),
        &identity,
        &session,
    )
    .unwrap();
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();
    let retained_operation = ProviderOperation::Llm;
    clock.set(201);

    let denied = turn.issue_provider_permit(
        &retained_operation.required_scope(),
        retained_operation.data_class(),
    );
    assert!(matches!(
        denied,
        Err(RuntimeDenyReason::AuthorizationExpired)
    ));
}

#[test]
fn policy_update_waits_for_execution_gate() {
    let session = Arc::new(active_session());
    let execution = session.gate.read().unwrap();
    let worker_session = Arc::clone(&session);
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();

    let worker = thread::spawn(move || {
        started_tx.send(()).unwrap();
        worker_session.set_consent(ConsentState::Revoked).unwrap();
        done_tx.send(()).unwrap();
    });

    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(done_rx.recv_timeout(Duration::from_millis(25)).is_err());
    drop(execution);
    done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    worker.join().unwrap();
}

#[test]
fn active_provider_permit_observes_lease_expiry() {
    let identity = persona();
    let clock = Arc::new(ManualClock::new(100));
    let (scope, authority) = allowed_provider_authority();
    let mut session = ActiveSession::with_clock(
        SessionId::new("session-stream-expiry").unwrap(),
        identity.id().clone(),
        SessionSecurityConfig::new(authority, Some(200), true, ConsentState::Granted, false),
        clock.clone(),
    );
    session.activate().unwrap();
    let turn = ActiveTurn::new(
        TurnId::new("turn-stream-expiry").unwrap(),
        CorrelationId::new("corr-stream-expiry").unwrap(),
        &identity,
        &session,
    )
    .unwrap();
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();
    let permit = turn
        .issue_provider_permit(&scope, DataClass::Personal)
        .unwrap();

    assert!(!CancellationProbe::is_cancelled(&permit.cancellation));
    clock.set(200);
    assert!(CancellationProbe::is_cancelled(&permit.cancellation));
}

#[test]
fn lease_expiry_blocks_late_output_evidence_and_completion() {
    let identity = persona();
    let clock = Arc::new(ManualClock::new(100));
    let (_, authority) = allowed_provider_authority();
    let mut session = ActiveSession::with_clock(
        SessionId::new("session-output-expiry").unwrap(),
        identity.id().clone(),
        SessionSecurityConfig::new(authority, Some(200), true, ConsentState::Granted, false),
        clock.clone(),
    );
    session.activate().unwrap();
    let turn = ActiveTurn::new(
        TurnId::new("turn-output-expiry").unwrap(),
        CorrelationId::new("corr-output-expiry").unwrap(),
        &identity,
        &session,
    )
    .unwrap();
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();
    turn.begin_output().unwrap();
    let segment = turn.begin_output_segment().unwrap();
    turn.mark_output_sent(segment).unwrap();

    clock.set(200);
    assert_eq!(
        turn.mark_output_played(segment),
        Err(Rt0ReasonCode::AuthExpired)
    );
    assert_eq!(turn.complete(), Err(Rt0ReasonCode::AuthExpired));
}

#[test]
fn provider_failures_map_to_stable_rt0_reason_codes() {
    assert_eq!(
        provider_reason_code(ProviderErrorKind::RateLimited),
        Rt0ReasonCode::ProviderRateLimited
    );
    assert_eq!(
        provider_reason_code(ProviderErrorKind::Cancelled),
        Rt0ReasonCode::TurnCancelled
    );
}
