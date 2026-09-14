mod support;

use std::io::Write;
use std::net::TcpListener;
use std::thread;

use vpr_domain::{
    CorrelationId, PersonaId, PersonaIdentity, PersonaMode, PersonaVersion, SessionId, TurnId,
};
use vpr_integration::{
    AudioInput, PcmSampleFormat, SttPort, SttRequest, Transcript, UsageEvidence, UsageUnit,
};
use vpr_policy::{AuthorityLayer, AuthorityScope, ConsentState, EffectiveAuthority};
use vpr_provider_deepgram_stt::{DeepgramStt, DeepgramSttConfig};
use vpr_provider_openai_transcription::{OpenAiTranscriptionConfig, OpenAiTranscriptionStt};
use vpr_runtime::{ActiveSession, ActiveTurn, SessionSecurityConfig};

fn serve_json_once(body: &'static str, path: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        support::read_complete_http_request(&mut stream);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
    });
    format!("http://{address}{path}")
}

fn request() -> SttRequest {
    SttRequest {
        audio: AudioInput {
            pcm: vec![0; 320],
            sample_rate_hz: 16_000,
            channels: 1,
            sample_format: PcmSampleFormat::S16Le,
        },
        locale_hint: Some("ru-RU".to_owned()),
    }
}

fn run_provider(port: &dyn SttPort) -> (Transcript, UsageEvidence) {
    let persona = PersonaIdentity::new(
        PersonaId::new("stt-contract-persona").unwrap(),
        PersonaVersion::new(1).unwrap(),
        PersonaMode::DigitalTwin,
    );
    let scope = AuthorityScope::new("provider.egress").unwrap();
    let authority = EffectiveAuthority::compose(&[AuthorityLayer::new([scope], [])]);
    let mut session = ActiveSession::new(
        SessionId::new("stt-contract-session").unwrap(),
        persona.id().clone(),
        SessionSecurityConfig::new(authority, None, true, ConsentState::Granted, false),
    );
    session.activate().unwrap();
    let turn = ActiveTurn::new(
        TurnId::new("stt-contract-turn").unwrap(),
        CorrelationId::new("stt-contract-correlation").unwrap(),
        &persona,
        &session,
    )
    .unwrap();
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();
    turn.execute_stt(port, &request()).unwrap()
}

#[test]
fn one_canonical_runtime_path_accepts_two_stt_protocols() {
    let openai_endpoint = serve_json_once(
        r#"{"text":"Привет","language":"ru-RU"}"#,
        "/v1/audio/transcriptions",
    );
    let deepgram_endpoint = serve_json_once(
        r#"{"results":{"channels":[{"alternatives":[{"transcript":"Привет","languages":["ru-RU"]}]}]}}"#,
        "/v1/listen",
    );
    let openai = OpenAiTranscriptionStt::new(OpenAiTranscriptionConfig::new(
        openai_endpoint,
        "secret",
        "stt-test",
    ))
    .unwrap();
    let deepgram = DeepgramStt::new(DeepgramSttConfig::new(
        deepgram_endpoint,
        "secret",
        "nova-test",
    ))
    .unwrap();

    let (openai_transcript, openai_usage) = run_provider(&openai);
    let (deepgram_transcript, deepgram_usage) = run_provider(&deepgram);

    for transcript in [&openai_transcript, &deepgram_transcript] {
        assert_eq!(transcript.text, "Привет");
        assert_eq!(transcript.locale, "ru-RU");
    }
    for usage in [&openai_usage, &deepgram_usage] {
        assert_eq!(usage.input_units, Some(10));
        assert_eq!(usage.input_unit, Some(UsageUnit::AudioMillisecond));
    }
    assert_eq!(openai.descriptor().provider, "openai-transcription");
    assert_eq!(deepgram.descriptor().provider, "deepgram");
}
