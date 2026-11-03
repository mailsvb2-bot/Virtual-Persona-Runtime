use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use tungstenite::{Message, accept};
use vpr_integration::{
    AudioInput, PcmSampleFormat, ProviderErrorKind, SttRequest, SttStreamEvent, SttStreamRequest,
    UsageUnit,
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

fn serve_live_once() -> (String, mpsc::Receiver<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (audio_tx, audio_rx) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut socket = accept(stream).unwrap();
        let Message::Binary(audio) = socket.read().unwrap() else {
            panic!("expected binary audio");
        };
        audio_tx.send(audio.to_vec()).unwrap();
        socket
            .send(Message::Text(
                r#"{"type":"Results","is_final":false,"channel":{"alternatives":[{"transcript":"Pri"}]}}"#
                    .into(),
            ))
            .unwrap();
        socket
            .send(Message::Text(
                r#"{"type":"Results","is_final":true,"channel":{"alternatives":[{"transcript":"Privet"}]}}"#
                    .into(),
            ))
            .unwrap();
        let Message::Text(close) = socket.read().unwrap() else {
            panic!("expected CloseStream control message");
        };
        assert_eq!(close, r#"{"type":"CloseStream"}"#);
        socket.close(None).unwrap();
    });
    (format!("http://{address}/v1/listen"), audio_rx)
}

#[test]
fn streams_live_pcm_and_normalizes_interim_and_final_events() {
    let (endpoint, captured_audio) = serve_live_once();
    let provider = adapter(endpoint);
    let probe = Probe(AtomicBool::new(false));
    let request = SttStreamRequest {
        sample_rate_hz: 16_000,
        channels: 1,
        sample_format: PcmSampleFormat::S16Le,
        locale_hint: Some("ru-RU".to_owned()),
    };
    let mut stream = provider.open_stream(&request, &probe).unwrap();
    let pcm = vec![0_u8; 640];
    stream.push_audio(&pcm, &probe).unwrap();

    assert_eq!(
        stream.next_event(&probe).unwrap().unwrap(),
        SttStreamEvent::Interim(Transcript {
            text: "Pri".to_owned(),
            locale: "ru".to_owned(),
        })
    );
    assert_eq!(
        stream.next_event(&probe).unwrap().unwrap(),
        SttStreamEvent::Final(Transcript {
            text: "Privet".to_owned(),
            locale: "ru".to_owned(),
        })
    );

    stream.finish_input(&probe).unwrap();
    assert_eq!(stream.usage().input_units, Some(20));
    assert_eq!(stream.usage().input_unit, Some(UsageUnit::AudioMillisecond));
    assert_eq!(captured_audio.recv().unwrap(), pcm);
}

#[test]
fn pre_cancelled_live_stream_never_connects() {
    let provider = adapter("http://127.0.0.1:1/v1/listen".to_owned());
    let probe = Probe(AtomicBool::new(true));
    let request = SttStreamRequest {
        sample_rate_hz: 16_000,
        channels: 1,
        sample_format: PcmSampleFormat::S16Le,
        locale_hint: Some("ru-RU".to_owned()),
    };
    let error = provider.open_stream(&request, &probe).err().unwrap();
    assert_eq!(error.kind, ProviderErrorKind::Cancelled);
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

#[test]
fn provider_http_authentication_denial_is_not_runtime_egress_denial() {
    for status in ["401 Unauthorized", "403 Forbidden"] {
        let (endpoint, _) = serve_once(status, "rejected");
        let provider = adapter(endpoint);
        let error = provider
            .transcribe(&request(), &Probe(AtomicBool::new(false)))
            .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::AuthenticationDenied);
        assert!(!error.retryable);
    }
}

#[test]
fn provider_websocket_authentication_denial_is_not_runtime_egress_denial() {
    for status in ["401 Unauthorized", "403 Forbidden"] {
        let (endpoint, _) = serve_once(status, "rejected");
        let provider = adapter(endpoint);
        let request = SttStreamRequest {
            sample_rate_hz: 16_000,
            channels: 1,
            sample_format: PcmSampleFormat::S16Le,
            locale_hint: Some("ru-RU".to_owned()),
        };
        let error = provider
            .open_stream(&request, &Probe(AtomicBool::new(false)))
            .err()
            .expect("WebSocket handshake must reject unauthorized provider credentials");
        assert_eq!(error.kind, ProviderErrorKind::AuthenticationDenied);
        assert!(!error.retryable);
    }
}

fn live_request() -> SttStreamRequest {
    SttStreamRequest {
        sample_rate_hz: 16_000,
        channels: 1,
        sample_format: PcmSampleFormat::S16Le,
        locale_hint: Some("ru-RU".to_owned()),
    }
}

/// A peer that accepts TCP but never answers the WebSocket upgrade must
/// not keep the visitor's canonical provider turn alive indefinitely.
#[test]
fn live_stt_handshake_stall_is_bounded() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1/listen", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        thread::sleep(std::time::Duration::from_millis(650));
    });
    let provider = DeepgramStt::new(
        DeepgramSttConfig::new(endpoint, "secret", "nova-test")
            .with_timeout(std::time::Duration::from_millis(250)),
    )
    .unwrap();
    let probe = Probe(AtomicBool::new(false));
    let started = std::time::Instant::now();
    let error = provider.open_stream(&live_request(), &probe).err().unwrap();
    assert_eq!(error.kind, ProviderErrorKind::Timeout);
    assert!(started.elapsed() < std::time::Duration::from_millis(550));
    server.join().unwrap();
}

/// Owner revoke must abort a pending WebSocket HTTP upgrade without waiting
/// for the provider's full configured timeout.
#[test]
fn live_stt_revoke_aborts_pending_handshake() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1/listen", listener.local_addr().unwrap());
    let (accepted_tx, accepted_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        accepted_tx.send(()).unwrap();
        thread::sleep(std::time::Duration::from_millis(650));
    });
    let provider = DeepgramStt::new(
        DeepgramSttConfig::new(endpoint, "secret", "nova-test")
            .with_timeout(std::time::Duration::from_secs(3)),
    )
    .unwrap();
    let probe = Probe(AtomicBool::new(false));
    thread::scope(|scope| {
        let pending = scope.spawn(|| provider.open_stream(&live_request(), &probe).err().unwrap());
        accepted_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let started = std::time::Instant::now();
        probe.0.store(true, Ordering::SeqCst);
        let error = pending.join().unwrap();
        assert_eq!(error.kind, ProviderErrorKind::Cancelled);
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
    });
    server.join().unwrap();
}

/// A silent WebSocket peer must not hold a canonical STT turn forever.
#[test]
fn live_stt_stalled_peer_has_bounded_event_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1/listen", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let _socket = accept(stream).unwrap();
        thread::sleep(std::time::Duration::from_millis(650));
    });
    let provider = DeepgramStt::new(
        DeepgramSttConfig::new(endpoint, "secret", "nova-test")
            .with_timeout(std::time::Duration::from_millis(250)),
    )
    .unwrap();
    let probe = Probe(AtomicBool::new(false));
    let mut stream = provider.open_stream(&live_request(), &probe).unwrap();
    let started = std::time::Instant::now();
    let error = stream.next_event(&probe).unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Timeout);
    assert!(started.elapsed() < std::time::Duration::from_millis(550));
    server.join().unwrap();
}

/// Revoke must win while the socket is waiting for a Deepgram transcript.
#[test]
fn live_stt_revocation_interrupts_silent_websocket() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1/listen", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let _socket = accept(stream).unwrap();
        thread::sleep(std::time::Duration::from_millis(600));
    });
    let provider = DeepgramStt::new(
        DeepgramSttConfig::new(endpoint, "secret", "nova-test")
            .with_timeout(std::time::Duration::from_secs(3)),
    )
    .unwrap();
    let probe = Probe(AtomicBool::new(false));
    let mut stream = provider.open_stream(&live_request(), &probe).unwrap();
    thread::scope(|scope| {
        let pending = scope.spawn(|| stream.next_event(&probe));
        thread::sleep(std::time::Duration::from_millis(120));
        let started = std::time::Instant::now();
        probe.0.store(true, Ordering::SeqCst);
        let error = pending.join().unwrap().unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::Cancelled);
        assert!(started.elapsed() < std::time::Duration::from_millis(350));
    });
    server.join().unwrap();
}

/// Provider-controlled websocket payloads are capped before buffering/JSON parsing.
#[test]
fn live_stt_rejects_oversized_websocket_transcript() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1/listen", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut socket = accept(stream).unwrap();
        let oversized = "x".repeat(vpr_integration::MAX_PROVIDER_STREAM_LINE_BYTES + 1);
        let _ = socket.send(Message::Text(oversized.into()));
    });
    let provider = adapter(endpoint);
    let probe = Probe(AtomicBool::new(false));
    let mut stream = provider.open_stream(&live_request(), &probe).unwrap();
    let error = stream.next_event(&probe).unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
    server.join().unwrap();
}

/// A 302 must not send the Deepgram Authorization header to another host.
#[test]
fn live_stt_websocket_redirect_is_not_followed() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1/listen", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        assert!(stream.read(&mut request).unwrap() > 0);
        stream.write_all(
            b"HTTP/1.1 302 Found\r\nLocation: ws://127.0.0.1:1/leak\r\nContent-Length: 0\r\n\r\n",
        ).unwrap();
    });
    let provider = adapter(endpoint);
    let probe = Probe(AtomicBool::new(false));
    let error = provider.open_stream(&live_request(), &probe).err().unwrap();
    assert_eq!(error.kind, ProviderErrorKind::PolicyDenied);
    assert!(!error.retryable);
    server.join().unwrap();
}
