use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use vpr_domain::{
    CorrelationId, OutputCheckpoint, OutputDeliveryState, PersonaId, PersonaIdentity, PersonaMode,
    PersonaVersion, Rt0ReasonCode, SessionId, TurnId, TurnState,
};
use vpr_integration::{
    CancellationProbe, GeneratedTextBuffer, GeneratedTextSink, LlmPort, LlmRequest,
    ProviderDescriptor, ProviderError, ProviderErrorKind, RealtimeOutputPort,
    RealtimeTextOutputEvent, TransportError, UsageEvidence,
};
use vpr_policy::{AuthorityLayer, AuthorityScope, ConsentState, EffectiveAuthority};
use vpr_runtime::{ActiveSession, ActiveTurn, ProviderExecutionError, SessionSecurityConfig};

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

struct BlockingLlm {
    started: mpsc::Sender<()>,
}

impl LlmPort for BlockingLlm {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider: "blocking-contract".into(),
            model: "blocking-contract".into(),
            representation: None,
        }
    }

    fn stream(
        &self,
        _request: &LlmRequest,
        cancellation: &dyn CancellationProbe,
        _sink: &mut dyn GeneratedTextSink,
    ) -> Result<UsageEvidence, ProviderError> {
        self.started.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !cancellation.is_cancelled() {
            assert!(
                Instant::now() < deadline,
                "in-flight provider never observed interruption"
            );
            thread::yield_now();
        }
        Err(ProviderError {
            kind: ProviderErrorKind::Cancelled,
            retryable: false,
        })
    }
}

fn active_turn() -> ActiveTurn {
    let persona = PersonaIdentity::new(
        PersonaId::new("concurrent-interrupt-persona").unwrap(),
        PersonaVersion::new(1).unwrap(),
        PersonaMode::DigitalTwin,
    );
    let scope = AuthorityScope::new("provider.egress").unwrap();
    let authority = EffectiveAuthority::compose(&[AuthorityLayer::new([scope], [])]);
    let mut session = ActiveSession::new(
        SessionId::new("concurrent-interrupt-session").unwrap(),
        persona.id().clone(),
        SessionSecurityConfig::new(authority, None, true, ConsentState::Granted, false),
    );
    session.activate().unwrap();
    let turn = ActiveTurn::new(
        TurnId::new("concurrent-interrupt-turn").unwrap(),
        CorrelationId::new("concurrent-interrupt-correlation").unwrap(),
        &persona,
        &session,
    )
    .unwrap();
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();
    turn.begin_output().unwrap();
    let played = turn.deliver_text(&ImmediateTransport, "played").unwrap();
    turn.acknowledge_output_played(&played).unwrap();
    let _tail = turn.deliver_text(&ImmediateTransport, "sent-tail").unwrap();
    turn
}

#[test]
fn interrupt_cancels_in_flight_provider_without_split_turn_state() {
    let turn = Arc::new(active_turn());
    let worker_turn = Arc::clone(&turn);
    let interrupt = turn.interrupt_handle();
    let (started_tx, started_rx) = mpsc::channel();

    let worker = thread::spawn(move || {
        let provider = BlockingLlm {
            started: started_tx,
        };
        worker_turn.execute_llm(
            &provider,
            &LlmRequest {
                locale: "ru-RU".into(),
                instructions: None,
                user_input: "stream until interrupted".into(),
            },
            &mut GeneratedTextBuffer::default(),
        )
    });

    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    interrupt.interrupt().unwrap();

    let result = worker.join().unwrap();
    assert!(matches!(
        result,
        Err(ProviderExecutionError::Provider(ProviderError {
            kind: ProviderErrorKind::Cancelled,
            retryable: false,
        }))
    ));
    assert_eq!(turn.state(), TurnState::Cancelled);
    assert_eq!(
        result.unwrap_err().reason_code(),
        Rt0ReasonCode::TurnCancelled
    );

    let segments = turn.output_segments();
    assert_eq!(segments.len(), 2);
    assert_eq!(
        segments[0].state(),
        OutputDeliveryState::Cancelled {
            reached: OutputCheckpoint::Played,
        }
    );
    assert!(segments[0].eligible_as_spoken());
    assert_eq!(
        segments[1].state(),
        OutputDeliveryState::Cancelled {
            reached: OutputCheckpoint::Sent,
        }
    );
    assert!(!segments[1].eligible_as_spoken());
}
