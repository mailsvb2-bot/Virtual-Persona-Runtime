use std::sync::atomic::{AtomicUsize, Ordering};

use vpr_domain::{
    CorrelationId, OutputDeliveryState, PersonaId, PersonaIdentity, PersonaMode, PersonaVersion,
    SessionId, TurnId, TurnState,
};
use vpr_integration::{
    CancellationProbe, ProviderDescriptor, ProviderError, RealtimeAvatarCapabilities,
    RealtimeAvatarCapability, RealtimeAvatarPort, RealtimeAvatarSession, WebRtcIceCandidate,
    WebRtcIceServer, WebRtcSessionDescription,
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
            provider_stream_id: "stream-secret".into(),
            provider_session_id: "provider-session-secret".into(),
            offer: WebRtcSessionDescription {
                kind: "offer".into(),
                sdp: "private-sdp".into(),
            },
            ice_servers: vec![WebRtcIceServer {
                urls: vec!["turn:example.invalid".into()],
                username: Some("u".into()),
                credential: Some("p".into()),
            }],
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
    assert_eq!(handle.offer().kind, "offer");
    assert_eq!(handle.ice_servers().len(), 1);
    second
        .speak_realtime_avatar_text(&provider, &handle, "Привет")
        .unwrap();

    assert_eq!(provider.creates.load(Ordering::SeqCst), 1);
    assert_eq!(provider.speaks.load(Ordering::SeqCst), 1);
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
    assert_eq!(output.output_segments()[0].state(), OutputDeliveryState::Sent);
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
