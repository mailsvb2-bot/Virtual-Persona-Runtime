use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use vpr_integration::{GeneratedAudioBuffer, ProviderErrorKind, TtsRequest, UsageUnit};

struct Probe(AtomicBool);
impl CancellationProbe for Probe {
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4_096];
    let mut total = None;
    loop {
        let read = stream.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if total.is_none()
            && let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
        {
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            });
            total = content_length.map(|length| header_end + 4 + length);
        }
        if total.is_some_and(|expected| request.len() >= expected) {
            break;
        }
    }
    request
}

fn serve_once(status: &str, body: Vec<u8>) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let status = status.to_owned();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_request(&mut stream);
        let _ = tx.send(String::from_utf8_lossy(&request).into_owned());
        let headers = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(headers.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
    });
    (format!("http://{address}/v1/text-to-speech"), rx)
}

fn adapter(endpoint: String) -> ElevenLabsTts {
    ElevenLabsTts::new(ElevenLabsTtsConfig::new(
        endpoint,
        "secret",
        "eleven-test",
        "voice-x",
    ))
    .unwrap()
}

fn request() -> TtsRequest {
    TtsRequest {
        text: "Привет".to_owned(),
        locale_hint: Some("ru-RU".to_owned()),
    }
}

#[test]
fn synthesizes_raw_pcm_and_reports_typed_usage() {
    let pcm = vec![0_u8; 480];
    let (endpoint, captured) = serve_once("200 OK", pcm.clone());
    let provider = adapter(endpoint);
    let mut sink = GeneratedAudioBuffer::default();
    let usage = provider
        .synthesize(&request(), &Probe(AtomicBool::new(false)), &mut sink)
        .unwrap();
    assert_eq!(sink.pcm(), pcm);
    assert_eq!(sink.sample_rate_hz(), Some(24_000));
    assert_eq!(sink.channels(), Some(1));
    assert_eq!(sink.sample_format(), Some(PcmSampleFormat::S16Le));
    assert_eq!(sink.duration_millis(), Some(10));
    assert_eq!(usage.input_units, Some(6));
    assert_eq!(usage.input_unit, Some(UsageUnit::TextCharacter));
    assert_eq!(usage.output_units, Some(10));
    assert_eq!(usage.output_unit, Some(UsageUnit::AudioMillisecond));
    let http = captured.recv().unwrap().to_ascii_lowercase();
    assert!(http.contains("/v1/text-to-speech/voice-x/stream?output_format=pcm_24000"));
    assert!(http.contains("xi-api-key: secret"));
    assert!(http.contains("\"model_id\":\"eleven-test\""));
    assert!(http.contains("\"language_code\":\"ru\""));
}

#[test]
fn pre_cancelled_request_never_starts_network_work() {
    let provider = adapter("http://127.0.0.1:1/v1/text-to-speech".to_owned());
    let error = provider
        .synthesize(
            &request(),
            &Probe(AtomicBool::new(true)),
            &mut GeneratedAudioBuffer::default(),
        )
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Cancelled);
}

#[test]
fn rejects_empty_text_before_network() {
    let provider = adapter("http://127.0.0.1:1/v1/text-to-speech".to_owned());
    let error = provider
        .synthesize(
            &TtsRequest {
                text: String::new(),
                locale_hint: None,
            },
            &Probe(AtomicBool::new(false)),
            &mut GeneratedAudioBuffer::default(),
        )
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
}

#[test]
fn rejects_partial_pcm_frame() {
    let (endpoint, _) = serve_once("200 OK", vec![1, 2, 3]);
    let error = adapter(endpoint)
        .synthesize(
            &request(),
            &Probe(AtomicBool::new(false)),
            &mut GeneratedAudioBuffer::default(),
        )
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
}

#[test]
fn maps_rate_limit_to_retryable_error() {
    let (endpoint, _) = serve_once("429 Too Many Requests", b"limited".to_vec());
    let error = adapter(endpoint)
        .synthesize(
            &request(),
            &Probe(AtomicBool::new(false)),
            &mut GeneratedAudioBuffer::default(),
        )
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::RateLimited);
    assert!(error.retryable);
}

#[test]
fn rejects_unsupported_pcm_rate() {
    let error = ElevenLabsTts::new(
        ElevenLabsTtsConfig::new("https://example.invalid/v1/text-to-speech", "key", "m", "v")
            .with_output_sample_rate_hz(12_345),
    )
    .err()
    .expect("unsupported PCM rate must fail");
    assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
}

#[test]
fn rejects_external_plain_http() {
    let error = ElevenLabsTts::new(ElevenLabsTtsConfig::new(
        "http://example.com/v1/text-to-speech",
        "secret",
        "model",
        "voice",
    ))
    .err()
    .expect("external plaintext must fail");
    assert_eq!(error.kind, ProviderErrorKind::PolicyDenied);
}

#[test]
fn descriptor_does_not_expose_secret() {
    let descriptor = ElevenLabsTtsConfig::new(
        "https://example.invalid/v1/text-to-speech",
        "super-secret",
        "model-x",
        "voice-x",
    )
    .descriptor();
    assert_eq!(descriptor.provider, "elevenlabs");
    assert_eq!(descriptor.model, "model-x");
    assert_eq!(descriptor.representation.as_deref(), Some("voice-x"));
}
