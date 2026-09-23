use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use vpr_integration::GeneratedTextBuffer;

struct Probe(AtomicBool);

impl CancellationProbe for Probe {
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

fn serve_once(status: &str, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let status = status.to_owned();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 8192];
        let _ = stream.read(&mut request).unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{body}"
        );
        stream.write_all(response.as_bytes()).unwrap();
    });
    format!("http://{address}/v1/chat/completions")
}

fn serve_once_capture(status: &str, body: &'static str) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let status = status.to_owned();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 8192];
        let read = stream.read(&mut request).unwrap();
        let _ = tx.send(String::from_utf8_lossy(&request[..read]).into_owned());
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{body}"
        );
        stream.write_all(response.as_bytes()).unwrap();
    });
    (format!("http://{address}/v1/chat/completions"), rx)
}

fn adapter(endpoint: String) -> OpenAiCompatibleLlm {
    OpenAiCompatibleLlm::new(OpenAiCompatibleConfig::new(
        endpoint,
        "secret",
        "test-model",
    ))
    .unwrap()
}

#[test]
fn optional_realtime_generation_controls_are_sent_only_when_configured() {
    let (endpoint, captured) = serve_once_capture("200 OK", "data: [DONE]\n\n");
    let provider = OpenAiCompatibleLlm::new(
        OpenAiCompatibleConfig::new(endpoint, "secret", "test-model")
            .with_reasoning_effort("none")
            .with_thinking_disabled()
            .with_max_tokens(96),
    )
    .unwrap();
    let probe = Probe(AtomicBool::new(false));
    provider
        .stream(
            &LlmRequest {
                locale: "ru-RU".into(),
                instructions: Some("canonical-policy".into()),
                user_input: "Коротко".into(),
            },
            &probe,
            &mut GeneratedTextBuffer::default(),
        )
        .unwrap();
    let request = captured.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(request.contains("\"max_tokens\":96"));
    assert!(request.contains("\"reasoning_effort\":\"none\""));
    assert!(request.contains("\"thinking\":{\"type\":\"disabled\"}"));
    assert!(request.contains("\"role\":\"system\",\"content\":\"canonical-policy\""));
    assert!(request.contains("\"role\":\"user\",\"content\":\"Коротко\""));
}

#[test]
fn streams_text_and_usage_from_openai_compatible_sse() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Привет\"}}],\"usage\":null}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"!\"}}],\"usage\":null}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":2}}\n\n",
        "data: [DONE]\n\n"
    );
    let provider = adapter(serve_once("200 OK", body));
    let probe = Probe(AtomicBool::new(false));
    let mut sink = GeneratedTextBuffer::default();
    let usage = provider
        .stream(
            &LlmRequest {
                locale: "ru-RU".into(),
                instructions: None,
                user_input: "Ответь кратко".into(),
            },
            &probe,
            &mut sink,
        )
        .unwrap();
    assert_eq!(sink.as_str(), "Привет!");
    assert_eq!(usage.input_units, Some(7));
    assert_eq!(usage.input_unit, Some(UsageUnit::Token));
    assert_eq!(usage.output_units, Some(2));
    assert_eq!(usage.output_unit, Some(UsageUnit::Token));
}

#[test]
fn pull_stream_exposes_early_chunks_and_final_usage() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Первая фраза.\"}}],\"usage\":null}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\" Вторая.\"}}],\"usage\":null}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":4}}\n\n",
        "data: [DONE]\n\n"
    );
    let provider = adapter(serve_once("200 OK", body));
    let probe = Probe(AtomicBool::new(false));
    let mut stream = provider
        .open_stream(
            &LlmRequest {
                locale: "ru-RU".into(),
                instructions: None,
                user_input: "Коротко".into(),
            },
            &probe,
        )
        .unwrap();
    assert_eq!(
        stream.next_chunk(&probe).unwrap().as_deref(),
        Some("Первая фраза.")
    );
    assert_eq!(
        stream.next_chunk(&probe).unwrap().as_deref(),
        Some(" Вторая.")
    );
    assert_eq!(stream.next_chunk(&probe).unwrap(), None);
    assert_eq!(stream.usage().input_units, Some(5));
    assert_eq!(stream.usage().output_units, Some(4));
}

#[test]
fn maps_rate_limit_to_typed_retryable_error() {
    let provider = adapter(serve_once("429 Too Many Requests", "rate limited"));
    let probe = Probe(AtomicBool::new(false));
    let error = provider
        .stream(
            &LlmRequest {
                locale: "ru-RU".into(),
                instructions: None,
                user_input: "test".into(),
            },
            &probe,
            &mut GeneratedTextBuffer::default(),
        )
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::RateLimited);
    assert!(error.retryable);
}

#[test]
fn pre_cancelled_request_never_requires_provider_success() {
    let probe = Probe(AtomicBool::new(true));
    let provider = adapter("http://127.0.0.1:1/v1/chat/completions".to_owned());
    let error = provider
        .stream(
            &LlmRequest {
                locale: "ru-RU".into(),
                instructions: None,
                user_input: "test".into(),
            },
            &probe,
            &mut GeneratedTextBuffer::default(),
        )
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Cancelled);
    assert!(!error.retryable);
}

#[test]
fn rejects_truncated_stream_without_done_marker() {
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}],\"usage\":null}\n\n";
    let provider = adapter(serve_once("200 OK", body));
    let probe = Probe(AtomicBool::new(false));
    let error = provider
        .stream(
            &LlmRequest {
                locale: "ru-RU".into(),
                instructions: None,
                user_input: "test".into(),
            },
            &probe,
            &mut GeneratedTextBuffer::default(),
        )
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
    assert!(!error.retryable);
}

#[test]
fn rejects_plain_http_for_non_loopback_endpoint() {
    let error = OpenAiCompatibleLlm::new(OpenAiCompatibleConfig::new(
        "http://example.com/v1/chat/completions",
        "secret",
        "model-x",
    ))
    .err()
    .expect("plain external HTTP must be rejected");
    assert_eq!(error.kind, ProviderErrorKind::PolicyDenied);
    assert!(!error.retryable);
}

#[test]
fn descriptor_does_not_expose_secret() {
    let config =
        OpenAiCompatibleConfig::new("https://example.invalid", "super-secret", "model-x");
    let descriptor = config.descriptor();
    assert_eq!(descriptor.provider, "openai-compatible");
    assert_eq!(descriptor.model, "model-x");
    assert_eq!(descriptor.representation, None);
}
