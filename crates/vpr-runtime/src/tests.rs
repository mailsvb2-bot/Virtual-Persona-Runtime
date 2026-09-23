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
use vpr_integration::{
    CancellationProbe, GeneratedTextSink, LlmPort, LlmRequest, LlmTextStream, PcmSampleFormat,
    ProviderDescriptor, ProviderError, ProviderErrorKind, SttAudioStream, SttPort, SttRequest,
    SttStreamEvent, SttStreamRequest, Transcript, UsageEvidence, UsageUnit,
};
use vpr_policy::{AuthorityLayer, AuthorityScope, ConsentState, DataClass, EffectiveAuthority};

struct PullLlm;

struct PullStream {
    emitted: bool,
}

impl LlmTextStream for PullStream {
    fn next_chunk(
        &mut self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<Option<String>, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(ProviderError {
                kind: ProviderErrorKind::Cancelled,
                retryable: false,
            });
        }
        if self.emitted {
            Ok(None)
        } else {
            self.emitted = true;
            Ok(Some("Первая фраза.".into()))
        }
    }

    fn usage(&self) -> UsageEvidence {
        UsageEvidence::default()
    }
}

impl LlmPort for PullLlm {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider: "pull-test".into(),
            model: "contract".into(),
            representation: None,
        }
    }

    fn open_stream(
        &self,
        _request: &LlmRequest,
        cancellation: &dyn CancellationProbe,
    ) -> Result<Box<dyn LlmTextStream>, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(ProviderError {
                kind: ProviderErrorKind::Cancelled,
                retryable: false,
            });
        }
        Ok(Box::new(PullStream { emitted: false }))
    }

    fn stream(
        &self,
        _request: &LlmRequest,
        _cancellation: &dyn CancellationProbe,
        _sink: &mut dyn GeneratedTextSink,
    ) -> Result<UsageEvidence, ProviderError> {
        unreachable!("runtime pull-stream test must not use callback streaming")
    }
}

struct PullStt;

struct PullSttStream {
    event_index: u8,
    audio_bytes: u64,
    input_finished: bool,
}

impl SttAudioStream for PullSttStream {
    fn push_audio(
        &mut self,
        pcm: &[u8],
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled_provider_error());
        }
        if self.input_finished || pcm.is_empty() || pcm.len() % 2 != 0 {
            return Err(ProviderError {
                kind: ProviderErrorKind::InvalidResponse,
                retryable: false,
            });
        }
        self.audio_bytes = self
            .audio_bytes
            .saturating_add(u64::try_from(pcm.len()).unwrap_or(u64::MAX));
        Ok(())
    }

    fn finish_input(&mut self, cancellation: &dyn CancellationProbe) -> Result<(), ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled_provider_error());
        }
        self.input_finished = true;
        Ok(())
    }

    fn next_event(
        &mut self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<Option<SttStreamEvent>, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled_provider_error());
        }
        let event = match self.event_index {
            0 => Some(SttStreamEvent::Interim(Transcript {
                text: "При".into(),
                locale: "ru".into(),
            })),
            1 if self.input_finished => Some(SttStreamEvent::Final(Transcript {
                text: "Привет".into(),
                locale: "ru".into(),
            })),
            1 => return Ok(None),
            _ => None,
        };
        if event.is_some() {
            self.event_index = self.event_index.saturating_add(1);
        }
        Ok(event)
    }

    fn usage(&self) -> UsageEvidence {
        UsageEvidence {
            input_units: Some(self.audio_bytes / 32),
            input_unit: Some(UsageUnit::AudioMillisecond),
            ..UsageEvidence::default()
        }
    }
}

impl SttPort for PullStt {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider: "pull-stt-test".into(),
            model: "contract".into(),
            representation: Some("streaming".into()),
        }
    }

    fn open_stream(
        &self,
        request: &SttStreamRequest,
        cancellation: &dyn CancellationProbe,
    ) -> Result<Box<dyn SttAudioStream>, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled_provider_error());
        }
        if !request.is_well_formed() {
            return Err(ProviderError {
                kind: ProviderErrorKind::InvalidResponse,
                retryable: false,
            });
        }
        Ok(Box::new(PullSttStream {
            event_index: 0,
            audio_bytes: 0,
            input_finished: false,
        }))
    }

    fn transcribe(
        &self,
        _request: &SttRequest,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<(Transcript, UsageEvidence), ProviderError> {
        unreachable!("runtime streaming STT test must not use batch transcription")
    }
}

fn cancelled_provider_error() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::Cancelled,
        retryable: false,
    }
}

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
fn authorized_stt_stream_keeps_one_permit_across_audio_and_transcript_events() {
    let session = active_session();
    let turn = turn(&session, "stt-stream");
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();
    let mut stream = turn
        .open_stt_stream(
            &PullStt,
            &SttStreamRequest {
                sample_rate_hz: 16_000,
                channels: 1,
                sample_format: PcmSampleFormat::S16Le,
                locale_hint: Some("ru-RU".into()),
            },
        )
        .unwrap();

    stream.push_audio(&[0; 640]).unwrap();
    let interim = stream.next_event().unwrap().unwrap();
    assert!(!interim.is_final());
    assert_eq!(interim.transcript().text, "При");

    stream.finish_input().unwrap();
    let final_event = stream.next_event().unwrap().unwrap();
    assert!(final_event.is_final());
    assert_eq!(final_event.transcript().text, "Привет");
    assert_eq!(stream.usage().input_units, Some(20));

    turn.interrupt_handle().interrupt().unwrap();
    let error = stream.next_event().unwrap_err();
    assert_eq!(error.reason_code(), Rt0ReasonCode::TurnCancelled);
}

#[test]
fn authorized_stt_stream_is_cancelled_by_session_revoke() {
    let mut session = active_session();
    let turn = turn(&session, "stt-stream-revoke");
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();
    let mut stream = turn
        .open_stt_stream(
            &PullStt,
            &SttStreamRequest {
                sample_rate_hz: 16_000,
                channels: 1,
                sample_format: PcmSampleFormat::S16Le,
                locale_hint: None,
            },
        )
        .unwrap();
    stream.push_audio(&[0; 320]).unwrap();

    session.revoke().unwrap();

    let error = stream.finish_input().unwrap_err();
    assert_eq!(error.reason_code(), Rt0ReasonCode::TurnCancelled);
}

#[test]
fn authorized_llm_stream_is_cancelled_by_turn_interrupt() {
    let session = active_session();
    let turn = turn(&session, "pull-stream-cancel");
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();
    let mut stream = turn
        .open_llm_stream(
            &PullLlm,
            &LlmRequest {
                locale: "ru-RU".into(),
                context: "Ответь кратко".into(),
            },
        )
        .unwrap();

    assert_eq!(
        stream.next_chunk().unwrap().as_deref(),
        Some("Первая фраза.")
    );
    turn.interrupt_handle().interrupt().unwrap();
    let error = stream.next_chunk().unwrap_err();
    assert_eq!(error.reason_code(), Rt0ReasonCode::TurnCancelled);
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
