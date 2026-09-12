use std::sync::atomic::{AtomicUsize, Ordering};

use vpr_domain::{
    CorrelationId, OutputCheckpoint, OutputDeliveryState, PersonaId, PersonaIdentity, PersonaMode,
    PersonaVersion, Rt0ReasonCode, SessionId, TurnId, TurnState,
};
use vpr_integration::{
    CancellationProbe, LlmPort, LlmRequest, ProviderDescriptor, ProviderError, TextSink,
    UsageEvidence,
};
use vpr_policy::{AuthorityLayer, AuthorityScope, ConsentState, DataClass, EffectiveAuthority};
use vpr_runtime::{
    ActiveSession, ActiveTurn, ProviderExecutionContext, ProviderExecutionError, RuntimeDenyReason,
    SessionSecurityConfig,
};

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
        sink: &mut dyn TextSink,
    ) -> Result<UsageEvidence, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(!cancellation.is_cancelled());
        sink.push_text("ok")?;
        Ok(UsageEvidence::default())
    }
}

#[derive(Default)]
struct TestSink(String);

impl TextSink for TestSink {
    fn push_text(&mut self, chunk: &str) -> Result<(), ProviderError> {
        self.0.push_str(chunk);
        Ok(())
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
    let mut turn = ActiveTurn::new(
        TurnId::new("turn-owner-test").unwrap(),
        CorrelationId::new("corr-owner-test").unwrap(),
        &persona,
        &session,
    )
    .unwrap();
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();

    let mut sink = TestSink::default();
    turn.execute_llm(
        ProviderExecutionContext::new(&provider_scope, DataClass::Biometric),
        &TestLlm::default(),
        &LlmRequest {
            locale: "ru-RU".into(),
            context: "test".into(),
        },
        &mut sink,
    )
    .unwrap();
    assert_eq!(sink.0, "ok");

    turn.begin_output().unwrap();
    let spoken = turn.begin_output_segment().unwrap();
    turn.mark_output_sent(spoken).unwrap();
    turn.mark_output_played(spoken).unwrap();
    let tail = turn.begin_output_segment().unwrap();
    turn.mark_output_sent(tail).unwrap();

    session.revoke().unwrap();
    let denied = turn.execute_llm(
        ProviderExecutionContext::new(&provider_scope, DataClass::Biometric),
        &TestLlm::default(),
        &LlmRequest {
            locale: "ru-RU".into(),
            context: "blocked".into(),
        },
        &mut TestSink::default(),
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
    let mut turn = ActiveTurn::new(
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
