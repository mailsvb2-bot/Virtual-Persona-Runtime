use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use vpr_integration::{
    AudioInput, PcmSampleFormat, ProviderErrorKind, SttRequest, SttStreamRequest, UsageUnit,
};

struct Probe(AtomicBool);
impl CancellationProbe for Probe {
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

fn request() -> SttRequest {
    SttRequest {
        audio: AudioInput {
            pcm: vec![0; 640],
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
        let mut bytes = [0_u8; 16_384];
        let read = stream.read(&mut bytes).unwrap();
        let _ = tx.send(String::from_utf8_lossy(&bytes[..read]).into_owned());
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
    });
    (format!("http://{address}/v1/listen"), rx)
}

fn adapter(endpoint: String) -> DeepgramStt {
    DeepgramStt::new(DeepgramSttConfig::new(endpoint, "secret", "nova-test")).unwrap()
}

#[test]
fn transcribes_raw_linear16_and_reports_duration() {
    let body = r#"{"results":{"channels":[{"alternatives":[{"transcript":"Привет","languages":["ru"]}]}]}}"#;
    let (endpoint, captured) = serve_once("200 OK", body);
    let provider = adapter(endpoint);
    let probe = Probe(AtomicBool::new(false));
    let (transcript, usage) = provider.transcribe(&request(), &probe).unwrap();
    assert_eq!(transcript.text, "Привет");
    assert_eq!(transcript.locale, "ru");
    assert_eq!(usage.input_units, Some(20));
    assert_eq!(usage.input_unit, Some(UsageUnit::AudioMillisecond));
    let http = captured.recv().unwrap().to_ascii_lowercase();
    assert!(http.contains("encoding=linear16"));
    assert!(http.contains("sample_rate=16000"));
    assert!(http.contains("channels=1"));
    assert!(http.contains("model=nova-test"));
    assert!(http.contains("language=ru"));
    assert!(!http.contains("language=ru-ru"));
    assert!(http.contains("authorization: token secret"));
}

#[test]
fn batch_adapter_does_not_claim_streaming_support() {
    let provider = adapter("http://127.0.0.1:1/v1/listen".to_owned());
    let probe = Probe(AtomicBool::new(false));
    let request = SttStreamRequest {
        sample_rate_hz: 16_000,
        channels: 1,
        sample_format: PcmSampleFormat::S16Le,
        locale_hint: Some("ru-RU".to_owned()),
    };
    let error = match provider.open_stream(&request, &probe) {
        Ok(_) => panic!("batch Deepgram adapter must not expose a fake streaming session"),
        Err(error) => error,
    };
    assert_eq!(error.kind, ProviderErrorKind::Unavailable);
    assert!(!error.retryable);
}

#[test]
fn pre_cancelled_request_never_starts_network_work() {
    let provider = adapter("http://127.0.0.1:1/v1/listen".to_owned());
    let probe = Probe(AtomicBool::new(true));
    let error = provider.transcribe(&request(), &probe).unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Cancelled);
}

#[test]
fn rejects_malformed_pcm_before_network() {
    let provider = adapter("http://127.0.0.1:1/v1/listen".to_owned());
    let probe = Probe(AtomicBool::new(false));
    let mut bad = request();
    bad.audio.channels = 0;
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
    let error = DeepgramStt::new(DeepgramSttConfig::new(
        "http://example.com/v1/listen",
        "secret",
        "nova-3",
    ))
    .err()
    .expect("external plaintext must fail");
    assert_eq!(error.kind, ProviderErrorKind::PolicyDenied);
}

#[test]
fn descriptor_does_not_expose_secret() {
    let config = DeepgramSttConfig::new("https://example.invalid", "super-secret", "nova-x");
    let descriptor = config.descriptor();
    assert_eq!(descriptor.provider, "deepgram");
    assert_eq!(descriptor.model, "nova-x");
}
