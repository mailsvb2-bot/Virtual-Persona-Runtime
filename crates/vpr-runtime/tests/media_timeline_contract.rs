use std::sync::Mutex;

use vpr_domain::{
    CorrelationId, PersonaId, PersonaIdentity, PersonaMode, PersonaVersion, Rt0ReasonCode,
    SessionId, TurnId, TurnState,
};
use vpr_integration::{
    CancellationProbe, GeneratedAudioBuffer, GeneratedAudioSink, GeneratedVideoFrame,
    PcmSampleFormat, RealtimeAudioOutputEvent, RealtimeMediaFlushEvent, RealtimeOutputPort,
    RealtimeTextOutputEvent, RealtimeVideoOutputEvent, TransportError,
};
use vpr_policy::{AuthorityLayer, AuthorityScope, ConsentState, EffectiveAuthority};
use vpr_runtime::{ActiveSession, ActiveTurn, OutputDeliveryError, SessionSecurityConfig};

#[derive(Default)]
struct RecordingMediaTransport {
    audio: Mutex<Vec<RealtimeAudioOutputEvent>>,
    video: Mutex<Vec<RealtimeVideoOutputEvent>>,
    flushes: Mutex<Vec<RealtimeMediaFlushEvent>>,
}

impl RealtimeOutputPort for RecordingMediaTransport {
    fn send_text(
        &self,
        _event: &RealtimeTextOutputEvent,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<(), TransportError> {
        unreachable!("media contract does not send text")
    }

    fn send_audio(
        &self,
        event: &RealtimeAudioOutputEvent,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), TransportError> {
        assert!(!cancellation.is_cancelled());
        self.audio.lock().unwrap().push(event.clone());
        Ok(())
    }

    fn send_video(
        &self,
        event: &RealtimeVideoOutputEvent,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), TransportError> {
        assert!(!cancellation.is_cancelled());
        self.video.lock().unwrap().push(event.clone());
        Ok(())
    }

    fn flush_media(&self, event: &RealtimeMediaFlushEvent) -> Result<(), TransportError> {
        self.flushes.lock().unwrap().push(event.clone());
        Ok(())
    }
}

fn setup(name: &str) -> (PersonaIdentity, ActiveSession, ActiveTurn) {
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
    let turn = start_turn(name, &persona, &session);
    (persona, session, turn)
}

fn start_turn(name: &str, persona: &PersonaIdentity, session: &ActiveSession) -> ActiveTurn {
    let turn = ActiveTurn::new(
        TurnId::new(format!("turn-{name}")).unwrap(),
        CorrelationId::new(format!("corr-{name}")).unwrap(),
        persona,
        session,
    )
    .unwrap();
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();
    turn.begin_output().unwrap();
    turn
}

fn audio_100ms() -> GeneratedAudioBuffer {
    let mut audio = GeneratedAudioBuffer::default();
    audio
        .push_generated_audio(&vec![0; 4_800], 24_000, 1, PcmSampleFormat::S16Le)
        .unwrap();
    audio
}

#[test]
fn audio_and_video_share_runtime_owned_origin_and_monotonic_sequence() {
    let (_persona, _session, turn) = setup("media-origin");
    let transport = RecordingMediaTransport::default();

    turn.deliver_audio(&transport, &audio_100ms()).unwrap();
    turn.deliver_video_frame(
        &transport,
        &GeneratedVideoFrame {
            encoded_frame: vec![1, 2],
            timestamp_micros: 9_000_000_000,
        },
    )
    .unwrap();
    turn.deliver_video_frame(
        &transport,
        &GeneratedVideoFrame {
            encoded_frame: vec![3, 4],
            timestamp_micros: 9_000_033_333,
        },
    )
    .unwrap();

    let audio = transport.audio.lock().unwrap();
    let video = transport.video.lock().unwrap();
    assert_eq!(audio.len(), 1);
    assert_eq!(video.len(), 2);
    assert_eq!(audio[0].session_id.as_str(), "session-media-origin");
    assert_eq!(audio[0].timeline.epoch, video[0].timeline.epoch);
    assert_eq!(
        audio[0].timeline.presentation_time_micros,
        video[0].timeline.presentation_time_micros
    );
    assert_eq!(audio[0].timeline.sequence + 1, video[0].timeline.sequence);
    assert_eq!(video[0].timeline.sequence + 1, video[1].timeline.sequence);
    assert_eq!(
        video[1].timeline.presentation_time_micros - video[0].timeline.presentation_time_micros,
        33_333
    );
}

#[test]
fn decreasing_provider_video_time_fails_before_transport() {
    let (_persona, _session, turn) = setup("media-regression");
    let transport = RecordingMediaTransport::default();
    turn.deliver_video_frame(
        &transport,
        &GeneratedVideoFrame {
            encoded_frame: vec![1],
            timestamp_micros: 500,
        },
    )
    .unwrap();

    let error = turn
        .deliver_video_frame(
            &transport,
            &GeneratedVideoFrame {
                encoded_frame: vec![2],
                timestamp_micros: 499,
            },
        )
        .unwrap_err();
    assert_eq!(
        error,
        OutputDeliveryError::Runtime(Rt0ReasonCode::InvalidStateTransition)
    );
    assert_eq!(transport.video.lock().unwrap().len(), 1);
    assert_eq!(turn.output_segments().len(), 1);
}

#[test]
fn session_rebase_stales_old_turn_mapping_and_new_turn_uses_new_epoch() {
    let (persona, session, old_turn) = setup("media-rebase");
    let transport = RecordingMediaTransport::default();
    old_turn
        .deliver_video_frame(
            &transport,
            &GeneratedVideoFrame {
                encoded_frame: vec![1],
                timestamp_micros: 100,
            },
        )
        .unwrap();
    let old_event = transport.video.lock().unwrap()[0].clone();

    let new_epoch = session.rebase_media_timeline().unwrap();
    assert_eq!(new_epoch, old_event.timeline.epoch + 1);
    let stale = old_turn
        .deliver_video_frame(
            &transport,
            &GeneratedVideoFrame {
                encoded_frame: vec![2],
                timestamp_micros: 200,
            },
        )
        .unwrap_err();
    assert_eq!(
        stale,
        OutputDeliveryError::Runtime(Rt0ReasonCode::InvalidStateTransition)
    );

    let new_turn = start_turn("media-rebase-new", &persona, &session);
    new_turn
        .deliver_video_frame(
            &transport,
            &GeneratedVideoFrame {
                encoded_frame: vec![3],
                timestamp_micros: 42_000_000,
            },
        )
        .unwrap();
    let events = transport.video.lock().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].timeline.epoch, new_epoch);
    assert!(events[1].timeline.sequence > old_event.timeline.sequence);
    assert!(
        events[1].timeline.presentation_time_micros > old_event.timeline.presentation_time_micros
    );
}

#[test]
fn interruption_flushes_through_last_allocated_media_sequence() {
    let (_persona, _session, turn) = setup("media-flush");
    let transport = RecordingMediaTransport::default();
    turn.deliver_audio(&transport, &audio_100ms()).unwrap();
    turn.deliver_video_frame(
        &transport,
        &GeneratedVideoFrame {
            encoded_frame: vec![9],
            timestamp_micros: 700,
        },
    )
    .unwrap();

    let last_sequence = transport.video.lock().unwrap()[0].timeline.sequence;
    turn.interrupt_and_flush_media(&transport).unwrap();
    assert_eq!(turn.state(), TurnState::Cancelled);
    let flushes = transport.flushes.lock().unwrap();
    assert_eq!(flushes.len(), 1);
    assert_eq!(flushes[0].turn_id.as_str(), "turn-media-flush");
    assert_eq!(flushes[0].through_sequence, Some(last_sequence));
    assert_eq!(
        flushes[0].epoch,
        transport.audio.lock().unwrap()[0].timeline.epoch
    );
}

#[test]
fn turn_created_before_rebase_is_stale_even_without_prior_media() {
    let (persona, session, old_turn) = setup("media-pre-rebase");
    let transport = RecordingMediaTransport::default();
    session.rebase_media_timeline().unwrap();

    let error = old_turn
        .deliver_audio(&transport, &audio_100ms())
        .unwrap_err();
    assert_eq!(
        error,
        OutputDeliveryError::Runtime(Rt0ReasonCode::InvalidStateTransition)
    );
    assert!(transport.audio.lock().unwrap().is_empty());

    let fresh = start_turn("media-pre-rebase-fresh", &persona, &session);
    fresh.deliver_audio(&transport, &audio_100ms()).unwrap();
    assert_eq!(transport.audio.lock().unwrap().len(), 1);
}

#[test]
fn empty_generated_media_is_rejected_before_transport() {
    let (_persona, _session, turn) = setup("media-empty");
    let transport = RecordingMediaTransport::default();
    assert_eq!(
        turn.deliver_audio(&transport, &GeneratedAudioBuffer::default()),
        Err(OutputDeliveryError::Runtime(Rt0ReasonCode::InternalError))
    );
    assert_eq!(
        turn.deliver_video_frame(
            &transport,
            &GeneratedVideoFrame {
                encoded_frame: Vec::new(),
                timestamp_micros: 0,
            },
        ),
        Err(OutputDeliveryError::Runtime(Rt0ReasonCode::InternalError))
    );
    assert!(transport.audio.lock().unwrap().is_empty());
    assert!(transport.video.lock().unwrap().is_empty());
}
