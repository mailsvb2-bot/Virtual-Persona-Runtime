use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use vpr_domain::{
    CorrelationId, OutputDeliveryState, PersonaId, PersonaIdentity, PersonaMode, PersonaVersion,
    SessionId, TurnId, TurnState,
};
use vpr_integration::{
    CancellationProbe, GeneratedAudioSink, PcmSampleFormat, ProviderDescriptor, ProviderError,
    ProviderErrorKind, RealtimeAvatarCapabilities,
    RealtimeAvatarCapability, RealtimeAvatarClientCommand, RealtimeAvatarClientControl,
    RealtimeAvatarClientEvent, RealtimeAvatarClientRoute, RealtimeAvatarPcmStream,
    RealtimeAvatarPort,
    RealtimeAvatarSession, RealtimeAvatarTransport, WebRtcIceCandidate, WebRtcIceServer,
    WebRtcSessionDescription, TtsPort, TtsRequest, UsageEvidence,
};
use vpr_policy::{AuthorityLayer, AuthorityScope, ConsentState, EffectiveAuthority};
use vpr_runtime::{
    ActiveSession, ActiveTurn, ProviderExecutionError, RuntimeDenyReason, SessionSecurityConfig,
};

#[derive(Default)]
struct RecordingAvatar {
    creates: AtomicUsize,
    speaks: AtomicUsize,
    closes: AtomicUsize,
    client_interrupts: AtomicUsize,
}

impl RecordingAvatar {
    fn descriptor_value() -> ProviderDescriptor {
        ProviderDescriptor {
            provider: "recording-avatar".into(),
            model: "realtime".into(),
            representation: Some("avatar-a".into()),
        }
    }
}

impl RealtimeAvatarPort for RecordingAvatar {
    fn descriptor(&self) -> ProviderDescriptor {
        Self::descriptor_value()
    }

    fn capabilities(&self) -> RealtimeAvatarCapabilities {
        RealtimeAvatarCapabilities::new([
            RealtimeAvatarCapability::TextInput,
            RealtimeAvatarCapability::Interrupt,
        ])
    }

    fn create_session(
        &self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarSession, ProviderError> {
        assert!(!cancellation.is_cancelled());
        self.creates.fetch_add(1, Ordering::SeqCst);
        Ok(RealtimeAvatarSession {
            provider_resource_id: "stream-secret".into(),
            provider_session_id: "provider-session-secret".into(),
            transport: RealtimeAvatarTransport::WebRtc {
                offer: WebRtcSessionDescription {
                    kind: "offer".into(),
                    sdp: "private-sdp".into(),
                },
                ice_servers: vec![WebRtcIceServer {
                    urls: vec!["turn:example.invalid".into()],
                    username: Some("u".into()),
                    credential: Some("p".into()),
                }],
            },
        })
    }

    fn submit_answer(
        &self,
        _session: &RealtimeAvatarSession,
        _answer: &WebRtcSessionDescription,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        assert!(!cancellation.is_cancelled());
        Ok(())
    }

    fn submit_ice_candidate(
        &self,
        _session: &RealtimeAvatarSession,
        _candidate: &WebRtcIceCandidate,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        assert!(!cancellation.is_cancelled());
        Ok(())
    }

    fn speak_text(
        &self,
        _session: &RealtimeAvatarSession,
        _text: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        assert!(!cancellation.is_cancelled());
        self.speaks.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn speak_audio_url(
        &self,
        _session: &RealtimeAvatarSession,
        _audio_url: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        assert!(!cancellation.is_cancelled());
        Ok(())
    }

    fn interrupt(
        &self,
        _session: &RealtimeAvatarSession,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        assert!(!cancellation.is_cancelled());
        Ok(())
    }

    fn client_control(
        &self,
        _session: &RealtimeAvatarSession,
    ) -> Option<RealtimeAvatarClientControl> {
        Some(RealtimeAvatarClientControl {
            event_route: Some(RealtimeAvatarClientRoute::WebRtcDataChannel {
                label: "test-channel".into(),
            }),
            interrupt: true,
            interrupt_requires_playback_id: true,
            text_input: false,
        })
    }

    fn parse_client_event(
        &self,
        _session: &RealtimeAvatarSession,
        message: &str,
    ) -> Result<Option<RealtimeAvatarClientEvent>, ProviderError> {
        Ok(
            (message == "started").then(|| RealtimeAvatarClientEvent::PlaybackStarted {
                playback_id: "playback-1".into(),
            }),
        )
    }

    fn prepare_client_interrupt(
        &self,
        _session: &RealtimeAvatarSession,
        playback_id: Option<&str>,
        cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarClientCommand, ProviderError> {
        assert!(!cancellation.is_cancelled());
        self.client_interrupts.fetch_add(1, Ordering::SeqCst);
        Ok(RealtimeAvatarClientCommand {
            route: RealtimeAvatarClientRoute::WebRtcDataChannel {
                label: "test-channel".into(),
            },
            payload: format!("interrupt:{}", playback_id.unwrap_or_default()),
        })
    }

    fn close_session(&self, _session: &RealtimeAvatarSession) -> Result<(), ProviderError> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn persona(name: &str) -> PersonaIdentity {
    PersonaIdentity::new(
        PersonaId::new(format!("persona-{name}")).unwrap(),
        PersonaVersion::new(1).unwrap(),
        PersonaMode::DigitalTwin,
    )
}

fn active_session(name: &str, persona: &PersonaIdentity) -> ActiveSession {
    let authority = EffectiveAuthority::compose(&[AuthorityLayer::new(
        [AuthorityScope::new("provider.egress").unwrap()],
        [],
    )]);
    let mut session = ActiveSession::new(
        SessionId::new(format!("session-{name}")).unwrap(),
        persona.id().clone(),
        SessionSecurityConfig::new(authority, None, true, ConsentState::Granted, false),
    );
    session.activate().unwrap();
    session
}

fn turn(name: &str, persona: &PersonaIdentity, session: &ActiveSession) -> ActiveTurn {
    let turn = ActiveTurn::new(
        TurnId::new(format!("turn-{name}")).unwrap(),
        CorrelationId::new(format!("corr-{name}")).unwrap(),
        persona,
        session,
    )
    .unwrap();
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();
    turn
}

#[test]
fn avatar_handle_is_session_scoped_and_survives_turn_boundary() {
    let persona = persona("shared");
    let session = active_session("shared", &persona);
    let first = turn("first", &persona, &session);
    let second = turn("second", &persona, &session);
    let provider = RecordingAvatar::default();

    let handle = first.open_realtime_avatar(&provider).unwrap();
    let RealtimeAvatarTransport::WebRtc { offer, ice_servers } = handle.transport() else {
        panic!("expected WebRTC transport");
    };
    assert_eq!(offer.kind, "offer");
    assert_eq!(ice_servers.len(), 1);
    second
        .speak_realtime_avatar_text(&provider, &handle, "Привет")
        .unwrap();

    assert_eq!(provider.creates.load(Ordering::SeqCst), 1);
    assert_eq!(provider.speaks.load(Ordering::SeqCst), 1);
}

#[test]
fn client_interrupt_command_is_exact_handle_and_authority_bound() {
    let persona = persona("client-control");
    let session = active_session("client-control", &persona);
    let opening = turn("client-open", &persona, &session);
    let provider = RecordingAvatar::default();
    let handle = opening.open_realtime_avatar(&provider).unwrap();
    assert_eq!(
        handle.client_control().unwrap().event_route,
        Some(RealtimeAvatarClientRoute::WebRtcDataChannel {
            label: "test-channel".into(),
        })
    );

    let control_turn = turn("client-control", &persona, &session);
    let event = control_turn
        .parse_realtime_avatar_client_event(&provider, &handle, "started")
        .unwrap();
    assert_eq!(
        event,
        Some(RealtimeAvatarClientEvent::PlaybackStarted {
            playback_id: "playback-1".into(),
        })
    );
    let command = control_turn
        .prepare_realtime_avatar_client_interrupt(&provider, &handle, Some("playback-1"))
        .unwrap();
    assert_eq!(
        command.route,
        RealtimeAvatarClientRoute::WebRtcDataChannel {
            label: "test-channel".into(),
        }
    );
    assert_eq!(command.payload, "interrupt:playback-1");
    assert_eq!(provider.client_interrupts.load(Ordering::SeqCst), 1);
}

#[test]
fn client_interrupt_command_rejects_cross_session_handle_before_provider() {
    let persona_a = persona("client-a");
    let session_a = active_session("client-a", &persona_a);
    let turn_a = turn("client-a", &persona_a, &session_a);
    let provider = RecordingAvatar::default();
    let handle = turn_a.open_realtime_avatar(&provider).unwrap();

    let persona_b = persona("client-b");
    let session_b = active_session("client-b", &persona_b);
    let turn_b = turn("client-b", &persona_b, &session_b);
    let error = turn_b
        .prepare_realtime_avatar_client_interrupt(&provider, &handle, Some("playback-1"))
        .unwrap_err();
    assert_eq!(
        error,
        ProviderExecutionError::Denied(RuntimeDenyReason::InvalidTurnState)
    );
    assert_eq!(provider.client_interrupts.load(Ordering::SeqCst), 0);
}

#[test]
fn avatar_text_delivery_is_sent_then_reconciled_to_played_after_turn_completion() {
    let persona = persona("delivery");
    let session = active_session("delivery", &persona);
    let opening = turn("delivery-open", &persona, &session);
    let provider = RecordingAvatar::default();
    let handle = opening.open_realtime_avatar(&provider).unwrap();

    let output = turn("delivery-output", &persona, &session);
    output.begin_output().unwrap();
    let delivery = output
        .deliver_realtime_avatar_text(&provider, &handle, "Привет")
        .unwrap();
    assert_eq!(delivery.sequence(), 1);
    assert_eq!(
        output.output_segments()[0].state(),
        OutputDeliveryState::Sent
    );
    output.complete().unwrap();
    assert_eq!(output.state(), TurnState::Completed);

    output.acknowledge_output_played(&delivery).unwrap();
    output.acknowledge_output_played(&delivery).unwrap();
    assert_eq!(
        output.output_segments()[0].state(),
        OutputDeliveryState::Played
    );
}

#[test]
fn cross_session_handle_is_rejected_before_provider_input() {
    let persona_a = persona("a");
    let session_a = active_session("a", &persona_a);
    let turn_a = turn("a", &persona_a, &session_a);
    let provider = RecordingAvatar::default();
    let handle = turn_a.open_realtime_avatar(&provider).unwrap();

    let persona_b = persona("b");
    let session_b = active_session("b", &persona_b);
    let turn_b = turn("b", &persona_b, &session_b);
    let error = turn_b
        .speak_realtime_avatar_text(&provider, &handle, "blocked")
        .unwrap_err();

    assert_eq!(
        error,
        ProviderExecutionError::Denied(RuntimeDenyReason::InvalidTurnState)
    );
    assert_eq!(provider.speaks.load(Ordering::SeqCst), 0);
}

#[test]
fn revoke_blocks_new_avatar_input_but_cleanup_remains_available() {
    let persona = persona("revoke");
    let mut session = active_session("revoke", &persona);
    let turn = turn("revoke", &persona, &session);
    let provider = RecordingAvatar::default();
    let mut handle = turn.open_realtime_avatar(&provider).unwrap();

    session.revoke().unwrap();
    assert!(matches!(
        turn.speak_realtime_avatar_text(&provider, &handle, "blocked"),
        Err(ProviderExecutionError::Denied(_))
    ));
    assert!(matches!(
        turn.prepare_realtime_avatar_client_interrupt(&provider, &handle, Some("playback-1")),
        Err(ProviderExecutionError::Denied(_))
    ));
    assert_eq!(provider.speaks.load(Ordering::SeqCst), 0);

    session
        .close_realtime_avatar(&provider, &mut handle)
        .unwrap();
    assert!(handle.is_closed());
    assert_eq!(provider.closes.load(Ordering::SeqCst), 1);

    session
        .close_realtime_avatar(&provider, &mut handle)
        .unwrap();
    assert_eq!(provider.closes.load(Ordering::SeqCst), 1);
}

#[test]
fn closed_handle_cannot_be_reused_by_a_later_turn() {
    let persona = persona("closed");
    let session = active_session("closed", &persona);
    let first = turn("closed-first", &persona, &session);
    let provider = RecordingAvatar::default();
    let mut handle = first.open_realtime_avatar(&provider).unwrap();
    session
        .close_realtime_avatar(&provider, &mut handle)
        .unwrap();

    let later = turn("closed-later", &persona, &session);
    let error = later
        .speak_realtime_avatar_text(&provider, &handle, "blocked")
        .unwrap_err();
    assert_eq!(
        error,
        ProviderExecutionError::Denied(RuntimeDenyReason::InvalidTurnState)
    );
    assert_eq!(provider.speaks.load(Ordering::SeqCst), 0);
}


#[derive(Default)]
struct StreamTrace {
    bytes: Mutex<Vec<u8>>,
    opens: AtomicUsize,
    chunks: AtomicUsize,
    finishes: AtomicUsize,
    aborts: AtomicUsize,
}

struct SimulatedPcmStream(Arc<StreamTrace>);
impl RealtimeAvatarPcmStream for SimulatedPcmStream {
    fn push_pcm(&mut self, pcm: &[u8], cancellation: &dyn CancellationProbe)
        -> Result<(), ProviderError> {
        if cancellation.is_cancelled() { return Err(provider_cancelled()); }
        self.0.bytes.lock().unwrap().extend_from_slice(pcm);
        self.0.chunks.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn finish(&mut self, cancellation: &dyn CancellationProbe) -> Result<(), ProviderError> {
        if cancellation.is_cancelled() { return Err(provider_cancelled()); }
        self.0.finishes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn abort(&mut self) -> Result<(), ProviderError> {
        self.0.aborts.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
fn provider_cancelled() -> ProviderError {
    ProviderError { kind: ProviderErrorKind::Cancelled, retryable: false }
}

struct NativePcmAvatar {
    fallback: RecordingAvatar,
    trace: Arc<StreamTrace>,
}
impl RealtimeAvatarPort for NativePcmAvatar {
    fn descriptor(&self) -> ProviderDescriptor { self.fallback.descriptor() }
    fn capabilities(&self) -> RealtimeAvatarCapabilities {
        RealtimeAvatarCapabilities::new([
            RealtimeAvatarCapability::AudioPcmInput,
            RealtimeAvatarCapability::AudioPcmStreamInput,
        ])
    }
    fn create_session(&self, probe: &dyn CancellationProbe)
        -> Result<RealtimeAvatarSession, ProviderError> {
        self.fallback.create_session(probe)
    }
    fn submit_answer(&self, session: &RealtimeAvatarSession,
        answer: &WebRtcSessionDescription, probe: &dyn CancellationProbe)
        -> Result<(), ProviderError> {
        self.fallback.submit_answer(session, answer, probe)
    }
    fn submit_ice_candidate(&self, session: &RealtimeAvatarSession,
        ice: &WebRtcIceCandidate, probe: &dyn CancellationProbe)
        -> Result<(), ProviderError> {
        self.fallback.submit_ice_candidate(session, ice, probe)
    }
    fn speak_text(&self, session: &RealtimeAvatarSession,
        text: &str, probe: &dyn CancellationProbe) -> Result<(), ProviderError> {
        self.fallback.speak_text(session, text, probe)
    }
    fn speak_audio_url(&self, session: &RealtimeAvatarSession,
        url: &str, probe: &dyn CancellationProbe) -> Result<(), ProviderError> {
        self.fallback.speak_audio_url(session, url, probe)
    }
    fn open_pcm_stream(&self, _session: &RealtimeAvatarSession,
        rate: u32, channels: u16, probe: &dyn CancellationProbe)
        -> Result<Box<dyn RealtimeAvatarPcmStream>, ProviderError> {
        if probe.is_cancelled() || rate != 24_000 || channels != 1 {
            return Err(provider_cancelled());
        }
        self.trace.opens.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(SimulatedPcmStream(Arc::clone(&self.trace))))
    }
    fn close_session(&self, session: &RealtimeAvatarSession) -> Result<(), ProviderError> {
        self.fallback.close_session(session)
    }
}

struct TwoChunkTts {
    trace: Arc<StreamTrace>,
    fail_after_first: bool,
}
impl TtsPort for TwoChunkTts {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor { provider: "two-chunk-tts".into(), model: "fake".into(),
            representation: None }
    }
    fn synthesize(&self, _request: &TtsRequest, probe: &dyn CancellationProbe,
        sink: &mut dyn GeneratedAudioSink) -> Result<UsageEvidence, ProviderError> {
        if probe.is_cancelled() { return Err(provider_cancelled()); }
        sink.push_generated_audio(&[1, 0, 2, 0], 24_000, 1, PcmSampleFormat::S16Le)?;
        assert_eq!(self.trace.chunks.load(Ordering::SeqCst), 1,
            "first generated PCM must reach private stream before full TTS finishes");
        if self.fail_after_first {
            return Err(ProviderError { kind: ProviderErrorKind::InvalidResponse, retryable: false });
        }
        sink.push_generated_audio(&[3, 0, 4, 0], 24_000, 1, PcmSampleFormat::S16Le)?;
        Ok(UsageEvidence::default())
    }
}

#[test]
fn canonical_pcm16_stream_delivers_first_audio_before_tts_finishes_without_claiming_playout() {
    let identity = persona("pcm-stream-pass");
    let session = active_session("pcm-stream-pass", &identity);
    let opening = turn("pcm-open", &identity, &session);
    let trace = Arc::new(StreamTrace::default());
    let avatar = NativePcmAvatar {
        fallback: RecordingAvatar::default(), trace: Arc::clone(&trace),
    };
    let handle = opening.open_realtime_avatar(&avatar).unwrap();
    let output = turn("pcm-output", &identity, &session);
    output.begin_output().unwrap();
    let tts = TwoChunkTts { trace: Arc::clone(&trace), fail_after_first: false };
    let delivery = output.synthesize_and_deliver_realtime_avatar_audio(
        &tts, &avatar, &handle, "Здравствуйте",
    ).unwrap();
    assert_eq!(trace.opens.load(Ordering::SeqCst), 1);
    assert_eq!(trace.chunks.load(Ordering::SeqCst), 2);
    assert_eq!(trace.finishes.load(Ordering::SeqCst), 1);
    assert_eq!(trace.aborts.load(Ordering::SeqCst), 0);
    assert_eq!(&*trace.bytes.lock().unwrap(), &[1, 0, 2, 0, 3, 0, 4, 0]);
    assert_eq!(output.output_segments()[0].state(), OutputDeliveryState::Sent);
    assert_eq!(delivery.sequence(), 1);
}

#[test]
fn partially_streamed_tts_failure_interrupts_avatar_and_never_promotes_sent_to_played() {
    let identity = persona("pcm-stream-fail");
    let session = active_session("pcm-stream-fail", &identity);
    let opening = turn("pcm-fail-open", &identity, &session);
    let trace = Arc::new(StreamTrace::default());
    let avatar = NativePcmAvatar {
        fallback: RecordingAvatar::default(), trace: Arc::clone(&trace),
    };
    let handle = opening.open_realtime_avatar(&avatar).unwrap();
    let output = turn("pcm-fail-output", &identity, &session);
    output.begin_output().unwrap();
    let tts = TwoChunkTts { trace: Arc::clone(&trace), fail_after_first: true };
    assert!(output.synthesize_and_deliver_realtime_avatar_audio(
        &tts, &avatar, &handle, "Здравствуйте",
    ).is_err());
    assert_eq!(trace.chunks.load(Ordering::SeqCst), 1);
    assert_eq!(trace.aborts.load(Ordering::SeqCst), 1);
    assert_eq!(trace.finishes.load(Ordering::SeqCst), 0);
    assert_ne!(output.output_segments()[0].state(), OutputDeliveryState::Sent);
    assert_ne!(output.output_segments()[0].state(), OutputDeliveryState::Played);
}
