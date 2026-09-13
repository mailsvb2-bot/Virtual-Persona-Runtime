use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use vpr_integration::{AudioInput, PcmSampleFormat, ProviderErrorKind, SttRequest, UsageUnit};

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

fn serve_once(status: &str, body: &'static str) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let status = status.to_owned();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_request(&mut stream);
        let _ = tx.send(String::from_utf8_lossy(&request).into_owned());
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
    });
    (format!("http://{address}/v1/audio/transcriptions"), rx)
}

fn adapter(endpoint: String) -> OpenAiTranscriptionStt {
    OpenAiTranscriptionStt::new(OpenAiTranscriptionConfig::new(
        endpoint,
        "secret",
        "stt-model",
    ))
    .unwrap()
}

#[test]
fn transcribes_pcm_via_wav_multipart_and_reports_duration() {
    let (endpoint, captured) = serve_once("200 OK", r#"{"text":"Привет","language":"ru"}"#);
    let provider = adapter(endpoint);
    let probe = Probe(AtomicBool::new(false));
    let (transcript, usage) = provider.transcribe(&request(), &probe).unwrap();
    assert_eq!(transcript.text, "Привет");
    assert_eq!(transcript.locale, "ru");
    assert_eq!(usage.input_units, Some(10));
    assert_eq!(usage.input_unit, Some(UsageUnit::AudioMillisecond));
    let http = captured.recv().unwrap();
    assert!(http.contains("/v1/audio/transcriptions"));
    assert!(http.contains("authorization: Bearer secret"));
    assert!(http.contains("name=\"model\""));
    assert!(http.contains("stt-model"));
    assert!(http.contains("filename=\"audio.wav\""));
    assert!(http.contains("RIFF"));
}

#[test]
fn pre_cancelled_request_never_starts_network_work() {
    let provider = adapter("http://127.0.0.1:1/v1/audio/transcriptions".to_owned());
    let probe = Probe(AtomicBool::new(true));
    let error = provider.transcribe(&request(), &probe).unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Cancelled);
    assert!(!error.retryable);
}

#[test]
fn rejects_malformed_pcm_before_network() {
    let provider = adapter("http://127.0.0.1:1/v1/audio/transcriptions".to_owned());
    let probe = Probe(AtomicBool::new(false));
    let mut bad = request();
    bad.audio.pcm.push(0);
    let error = provider.transcribe(&bad, &probe).unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
}

#[test]
fn maps_rate_limit_to_retryable_error() {
    let (endpoint, _) = serve_once("429 Too Many Requests", "rate limited");
    let provider = adapter(endpoint);
    let probe = Probe(AtomicBool::new(false));
    let error = provider.transcribe(&request(), &probe).unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::RateLimited);
    assert!(error.retryable);
}

#[test]
fn rejects_external_plain_http() {
    let error = OpenAiTranscriptionStt::new(OpenAiTranscriptionConfig::new(
        "http://example.com/v1/audio/transcriptions",
        "secret",
        "model",
    ))
    .err()
    .expect("external plaintext must fail");
    assert_eq!(error.kind, ProviderErrorKind::PolicyDenied);
}

#[test]
fn descriptor_does_not_expose_secret() {
    let config =
        OpenAiTranscriptionConfig::new("https://example.invalid", "super-secret", "model-x");
    let descriptor = config.descriptor();
    assert_eq!(descriptor.provider, "openai-transcription");
    assert_eq!(descriptor.model, "model-x");
}
