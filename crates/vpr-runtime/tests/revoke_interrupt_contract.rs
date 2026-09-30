use std::sync::atomic::{AtomicUsize, Ordering};

use vpr_domain::{
    CorrelationId, OutputCheckpoint, OutputDeliveryState, PersonaId, PersonaIdentity, PersonaMode,
    PersonaVersion, Rt0ReasonCode, SessionId, TurnId, TurnState,
};
use vpr_integration::{
    AudioInput, CancellationProbe, GeneratedAudioBuffer, GeneratedAudioSink, GeneratedTextBuffer,
    GeneratedTextSink, LlmPort, LlmRequest, PcmSampleFormat, ProviderDescriptor, ProviderError,
    RealtimeOutputPort, RealtimeTextOutputEvent, SttPort, SttRequest, Transcript, TransportError,
    TtsPort, TtsRequest, UsageEvidence,
};
use vpr_policy::{AuthorityLayer, AuthorityScope, ConsentState, EffectiveAuthority};
use vpr_runtime::{
    ActiveSession, ActiveTurn, ProviderExecutionError, RuntimeDenyReason, SessionSecurityConfig,
};

struct ImmediateTransport;

impl RealtimeOutputPort for ImmediateTransport {
    fn send_text(
        &self,
        _event: &RealtimeTextOutputEvent,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), TransportError> {
        assert!(!cancellation.is_cancelled());
        Ok(())
    }
}

#[derive(Default)]
struct TestLlm {
    calls: AtomicUsize,
}

impl LlmPort for TestLlm {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider: "test".into(),
            model: "test".into(),
            representation: None,
        }
    }

    fn stream(
        &self,
        _request: &LlmRequest,
        cancellation: &dyn CancellationProbe,
        sink: &mut dyn GeneratedTextSink,
    ) -> Result<UsageEvidence, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(!cancellation.is_cancelled());
        sink.push_generated_text("ok")?;
        Ok(UsageEvidence::default())
    }
}

#[derive(Default)]
struct TestTts {
    calls: AtomicUsize,
}

#[derive(Default)]
struct TestStt {
    calls: AtomicUsize,
}

impl SttPort for TestStt {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider: "test".into(),
            model: "test-stt".into(),
            representation: Some("speech-to-text".into()),
        }
    }

    fn transcribe(
        &self,
        request: &SttRequest,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(Transcript, UsageEvidence), ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(!cancellation.is_cancelled());
        Ok((
            Transcript {
                text: "ok".into(),
                locale: request.locale_hint.clone().unwrap_or_else(|| "und".into()),
            },
            UsageEvidence::default(),
        ))
    }
}

impl TtsPort for TestTts {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider: "test".into(),
            model: "test-tts".into(),
            representation: Some("voice-test".into()),
        }
    }

    fn synthesize(
        &self,
        _request: &TtsRequest,
        _cancellation: &dyn CancellationProbe,
        sink: &mut dyn GeneratedAudioSink,
    ) -> Result<UsageEvidence, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        sink.push_generated_audio(&[0, 0], 16_000, 1, PcmSampleFormat::S16Le)?;
        Ok(UsageEvidence::default())
    }
}

#[test]
fn session_revoke_during_stream_blocks_new_egress_and_preserves_spoken_prefix_only() {
    let persona = PersonaIdentity::new(
        PersonaId::new("persona-owner").unwrap(),
        PersonaVersion::new(1).unwrap(),
        PersonaMode::DigitalTwin,
    );
    let provider_scope = AuthorityScope::new("provider.egress").unwrap();
    let authority =
        EffectiveAuthority::compose(&[AuthorityLayer::new([provider_scope.clone()], [])]);
    let mut session = ActiveSession::new(
        SessionId::new("session-owner-test").unwrap(),
        persona.id().clone(),
        SessionSecurityConfig::new(authority, None, true, ConsentState::Granted, false),
    );
    session.activate().unwrap();
    let turn = ActiveTurn::new(
        TurnId::new("turn-owner-test").unwrap(),
        CorrelationId::new("corr-owner-test").unwrap(),
        &persona,
        &session,
    )
    .unwrap();
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();

    let mut sink = GeneratedTextBuffer::default();
    turn.execute_llm(
        &TestLlm::default(),
        &LlmRequest {
            locale: "ru-RU".into(),
            instructions: None,
            user_input: "test".into(),
        },
        &mut sink,
    )
    .unwrap();
    assert_eq!(sink.as_str(), "ok");

    turn.begin_output().unwrap();
    let spoken = turn.deliver_text(&ImmediateTransport, "spoken").unwrap();
    turn.acknowledge_output_played(&spoken).unwrap();
    let _tail = turn.deliver_text(&ImmediateTransport, "sent-tail").unwrap();

    session.revoke().unwrap();
    let denied = turn.execute_llm(
        &TestLlm::default(),
        &LlmRequest {
            locale: "ru-RU".into(),
            instructions: None,
            user_input: "blocked".into(),
        },
        &mut GeneratedTextBuffer::default(),
    );
    assert!(matches!(
        denied,
        Err(ProviderExecutionError::Denied(
            RuntimeDenyReason::AuthorizationStale
        ))
    ));
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

#[test]
fn unstructured_llm_context_is_biometric_fail_closed_without_consent() {
    let persona = PersonaIdentity::new(
        PersonaId::new("persona-llm-consent").unwrap(),
        PersonaVersion::new(1).unwrap(),
        PersonaMode::DigitalTwin,
    );
    let provider_scope = AuthorityScope::new("provider.egress").unwrap();
    let authority = EffectiveAuthority::compose(&[AuthorityLayer::new([provider_scope], [])]);
    let mut session = ActiveSession::new(
        SessionId::new("session-llm-consent").unwrap(),
        persona.id().clone(),
        SessionSecurityConfig::new(authority, None, true, ConsentState::Missing, false),
    );
    session.activate().unwrap();
    let turn = ActiveTurn::new(
        TurnId::new("turn-llm-consent").unwrap(),
        CorrelationId::new("corr-llm-consent").unwrap(),
        &persona,
        &session,
    )
    .unwrap();
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();

    let provider = TestLlm::default();
    let result = turn.execute_llm(
        &provider,
        &LlmRequest {
            locale: "ru-RU".into(),
            instructions: None,
            user_input: "arbitrary unstructured context".into(),
        },
        &mut GeneratedTextBuffer::default(),
    );
    assert!(matches!(
        result,
        Err(ProviderExecutionError::Denied(
            RuntimeDenyReason::ConsentRequired
        ))
    ));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn missing_consent_blocks_tts_before_adapter_start() {
    let persona = PersonaIdentity::new(
        PersonaId::new("persona-tts-consent").unwrap(),
        PersonaVersion::new(1).unwrap(),
        PersonaMode::DigitalTwin,
    );
    let provider_scope = AuthorityScope::new("provider.egress").unwrap();
    let authority = EffectiveAuthority::compose(&[AuthorityLayer::new([provider_scope], [])]);
    let mut session = ActiveSession::new(
        SessionId::new("session-tts-consent").unwrap(),
        persona.id().clone(),
        SessionSecurityConfig::new(authority, None, true, ConsentState::Missing, false),
    );
    session.activate().unwrap();
    let turn = ActiveTurn::new(
        TurnId::new("turn-tts-consent").unwrap(),
        CorrelationId::new("corr-tts-consent").unwrap(),
        &persona,
        &session,
    )
    .unwrap();
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();

    let provider = TestTts::default();
    let request = TtsRequest {
        text: "hello".into(),
        locale_hint: Some("en".into()),
    };
    let result = turn.execute_tts(&provider, &request, &mut GeneratedAudioBuffer::default());
    assert!(matches!(
        result,
        Err(ProviderExecutionError::Denied(
            RuntimeDenyReason::ConsentRequired
        ))
    ));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn expired_authority_fails_closed_before_provider_start() {
    let persona = PersonaIdentity::new(
        PersonaId::new("persona-expiry").unwrap(),
        PersonaVersion::new(1).unwrap(),
        PersonaMode::DigitalTwin,
    );
    let provider_scope = AuthorityScope::new("provider.egress").unwrap();
    let authority = EffectiveAuthority::compose(&[AuthorityLayer::new([provider_scope], [])]);
    let mut session = ActiveSession::new(
        SessionId::new("session-expiry").unwrap(),
        persona.id().clone(),
        SessionSecurityConfig::new(authority, Some(0), true, ConsentState::Granted, false),
    );
    session.activate().unwrap();
    let turn = ActiveTurn::new(
        TurnId::new("turn-expiry").unwrap(),
        CorrelationId::new("corr-expiry").unwrap(),
        &persona,
        &session,
    )
    .unwrap();
    let provider = TestLlm::default();
    assert_eq!(turn.authorize(), Err(Rt0ReasonCode::AuthExpired));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn missing_consent_blocks_stt_before_adapter_start() {
    let persona = PersonaIdentity::new(
        PersonaId::new("persona-stt-consent").unwrap(),
        PersonaVersion::new(1).unwrap(),
        PersonaMode::DigitalTwin,
    );
    let provider_scope = AuthorityScope::new("provider.egress").unwrap();
    let authority = EffectiveAuthority::compose(&[AuthorityLayer::new([provider_scope], [])]);
    let mut session = ActiveSession::new(
        SessionId::new("session-stt-consent").unwrap(),
        persona.id().clone(),
        SessionSecurityConfig::new(authority, None, true, ConsentState::Missing, false),
    );
    session.activate().unwrap();
    let turn = ActiveTurn::new(
        TurnId::new("turn-stt-consent").unwrap(),
        CorrelationId::new("corr-stt-consent").unwrap(),
        &persona,
        &session,
    )
    .unwrap();
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();

    let provider = TestStt::default();
    let request = SttRequest {
        audio: AudioInput {
            pcm: vec![0; 320],
            sample_rate_hz: 16_000,
            channels: 1,
            sample_format: PcmSampleFormat::S16Le,
        },
        locale_hint: Some("ru-RU".into()),
    };
    let error = turn.execute_stt(&provider, &request).unwrap_err();
    assert_eq!(error.reason_code(), Rt0ReasonCode::ConsentRequired);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}
