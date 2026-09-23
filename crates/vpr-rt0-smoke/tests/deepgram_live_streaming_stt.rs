use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use tungstenite::{Message, accept};
use vpr_domain::{
    CorrelationId, PersonaId, PersonaIdentity, PersonaMode, PersonaVersion, SessionId, TurnId,
};
use vpr_integration::{PcmSampleFormat, SttStreamEvent, SttStreamRequest, UsageUnit};
use vpr_policy::{AuthorityLayer, AuthorityScope, ConsentState, EffectiveAuthority};
use vpr_provider_deepgram_stt::{DeepgramStt, DeepgramSttConfig};
use vpr_runtime::{ActiveSession, ActiveTurn, SessionSecurityConfig};

fn live_provider() -> (DeepgramStt, mpsc::Receiver<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (audio_tx, audio_rx) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut websocket = accept(stream).unwrap();
        let Message::Binary(audio) = websocket.read().unwrap() else {
            panic!("expected streamed PCM");
        };
        audio_tx.send(audio.to_vec()).unwrap();
        websocket
            .send(Message::Text(
                r#"{"type":"Results","is_final":false,"channel":{"alternatives":[{"transcript":"Pri"}]}}"#
                    .into(),
            ))
            .unwrap();
        websocket
            .send(Message::Text(
                r#"{"type":"Results","is_final":true,"channel":{"alternatives":[{"transcript":"Privet"}]}}"#
                    .into(),
            ))
            .unwrap();
        let Message::Text(close) = websocket.read().unwrap() else {
            panic!("expected CloseStream");
        };
        assert_eq!(close, r#"{"type":"CloseStream"}"#);
        websocket.close(None).unwrap();
    });
    let provider = DeepgramStt::new(DeepgramSttConfig::new(
        format!("http://{address}/v1/listen"),
        "secret",
        "nova-test",
    ))
    .unwrap();
    (provider, audio_rx)
}

fn active_turn() -> (ActiveSession, ActiveTurn) {
    let persona = PersonaIdentity::new(
        PersonaId::new("deepgram-live-persona").unwrap(),
        PersonaVersion::new(1).unwrap(),
        PersonaMode::DigitalTwin,
    );
    let scope = AuthorityScope::new("provider.egress").unwrap();
    let authority = EffectiveAuthority::compose(&[AuthorityLayer::new([scope], [])]);
    let mut session = ActiveSession::new(
        SessionId::new("deepgram-live-session").unwrap(),
        persona.id().clone(),
        SessionSecurityConfig::new(authority, None, true, ConsentState::Granted, false),
    );
    session.activate().unwrap();
    let turn = ActiveTurn::new(
        TurnId::new("deepgram-live-turn").unwrap(),
        CorrelationId::new("deepgram-live-correlation").unwrap(),
        &persona,
        &session,
    )
    .unwrap();
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();
    (session, turn)
}

#[test]
fn canonical_runtime_authority_drives_real_deepgram_websocket_stream() {
    let (provider, captured_audio) = live_provider();
    let (_session, turn) = active_turn();
    let request = SttStreamRequest {
        sample_rate_hz: 16_000,
        channels: 1,
        sample_format: PcmSampleFormat::S16Le,
        locale_hint: Some("ru-RU".to_owned()),
    };
    let mut stream = turn.open_stt_stream(&provider, &request).unwrap();
    let pcm = vec![0_u8; 640];
    stream.push_audio(&pcm).unwrap();

    let interim = stream.next_event().unwrap().unwrap();
    assert!(matches!(interim, SttStreamEvent::Interim(_)));
    assert_eq!(interim.transcript().text, "Pri");
    assert_eq!(interim.transcript().locale, "ru");

    let final_event = stream.next_event().unwrap().unwrap();
    assert!(final_event.is_final());
    assert_eq!(final_event.transcript().text, "Privet");

    stream.finish_input().unwrap();
    assert_eq!(stream.usage().input_units, Some(20));
    assert_eq!(stream.usage().input_unit, Some(UsageUnit::AudioMillisecond));
    assert_eq!(captured_audio.recv().unwrap(), pcm);
}
