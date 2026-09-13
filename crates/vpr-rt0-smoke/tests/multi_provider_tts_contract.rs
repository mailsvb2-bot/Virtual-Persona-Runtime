use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use vpr_domain::{
    CorrelationId, PersonaId, PersonaIdentity, PersonaMode, PersonaVersion, SessionId, TurnId,
};
use vpr_integration::{
    GeneratedAudioBuffer, PcmSampleFormat, TtsPort, TtsRequest, UsageEvidence, UsageUnit,
};
use vpr_policy::{AuthorityLayer, AuthorityScope, ConsentState, EffectiveAuthority};
use vpr_provider_elevenlabs_tts::{ElevenLabsTts, ElevenLabsTtsConfig};
use vpr_provider_openai_speech::{OpenAiSpeechConfig, OpenAiSpeechTts};
use vpr_runtime::{ActiveSession, ActiveTurn, SessionSecurityConfig};

fn wav(pcm: &[u8], sample_rate: u32, channels: u16) -> Vec<u8> {
    let data_len = u32::try_from(pcm.len()).unwrap();
    let block_align = channels * 2;
    let mut output = Vec::new();
    output.extend_from_slice(b"RIFF");
    output.extend_from_slice(&(36 + data_len).to_le_bytes());
    output.extend_from_slice(b"WAVEfmt ");
    output.extend_from_slice(&16_u32.to_le_bytes());
    output.extend_from_slice(&1_u16.to_le_bytes());
    output.extend_from_slice(&channels.to_le_bytes());
    output.extend_from_slice(&sample_rate.to_le_bytes());
    output.extend_from_slice(&(sample_rate * u32::from(block_align)).to_le_bytes());
    output.extend_from_slice(&block_align.to_le_bytes());
    output.extend_from_slice(&16_u16.to_le_bytes());
    output.extend_from_slice(b"data");
    output.extend_from_slice(&data_len.to_le_bytes());
    output.extend_from_slice(pcm);
    output
}

fn serve_once(body: Vec<u8>, content_type: &'static str, path: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 16_384];
        let _ = stream.read(&mut request).unwrap();
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(headers.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
    });
    format!("http://{address}{path}")
}

fn run_provider(port: &dyn TtsPort) -> (GeneratedAudioBuffer, UsageEvidence) {
    let persona = PersonaIdentity::new(
        PersonaId::new("tts-contract-persona").unwrap(),
        PersonaVersion::new(1).unwrap(),
        PersonaMode::DigitalTwin,
    );
    let scope = AuthorityScope::new("provider.egress").unwrap();
    let authority = EffectiveAuthority::compose(&[AuthorityLayer::new([scope], [])]);
    let mut session = ActiveSession::new(
        SessionId::new("tts-contract-session").unwrap(),
        persona.id().clone(),
        SessionSecurityConfig::new(authority, None, true, ConsentState::Granted, false),
    );
    session.activate().unwrap();
    let mut turn = ActiveTurn::new(
        TurnId::new("tts-contract-turn").unwrap(),
        CorrelationId::new("tts-contract-correlation").unwrap(),
        &persona,
        &session,
    )
    .unwrap();
    turn.authorize().unwrap();
    turn.begin_processing().unwrap();
    let mut sink = GeneratedAudioBuffer::default();
    let usage = turn
        .execute_tts(
            port,
            &TtsRequest {
                text: "Привет".into(),
                locale_hint: Some("ru-RU".into()),
            },
            &mut sink,
        )
        .unwrap();
    (sink, usage)
}

#[test]
fn one_canonical_runtime_path_accepts_two_tts_protocols() {
    let pcm = vec![0_u8; 480];
    let openai_endpoint = serve_once(wav(&pcm, 24_000, 1), "audio/wav", "/v1/audio/speech");
    let eleven_endpoint = serve_once(
        pcm.clone(),
        "application/octet-stream",
        "/v1/text-to-speech",
    );
    let openai = OpenAiSpeechTts::new(OpenAiSpeechConfig::new(
        openai_endpoint,
        "secret",
        "tts-test",
        "voice-test",
    ))
    .unwrap();
    let eleven = ElevenLabsTts::new(ElevenLabsTtsConfig::new(
        eleven_endpoint,
        "secret",
        "eleven-test",
        "voice-test",
    ))
    .unwrap();

    let (openai_audio, openai_usage) = run_provider(&openai);
    let (eleven_audio, eleven_usage) = run_provider(&eleven);

    for audio in [&openai_audio, &eleven_audio] {
        assert_eq!(audio.pcm(), pcm);
        assert_eq!(audio.sample_rate_hz(), Some(24_000));
        assert_eq!(audio.channels(), Some(1));
        assert_eq!(audio.sample_format(), Some(PcmSampleFormat::S16Le));
        assert_eq!(audio.duration_millis(), Some(10));
    }
    for usage in [&openai_usage, &eleven_usage] {
        assert_eq!(usage.input_units, Some(6));
        assert_eq!(usage.input_unit, Some(UsageUnit::TextCharacter));
        assert_eq!(usage.output_units, Some(10));
        assert_eq!(usage.output_unit, Some(UsageUnit::AudioMillisecond));
    }
    assert_eq!(openai.descriptor().provider, "openai-speech");
    assert_eq!(eleven.descriptor().provider, "elevenlabs");
}
