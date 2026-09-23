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
    format!("http://{address}/v1beta/interactions?alt=sse")
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
        tx.send(String::from_utf8_lossy(&request[..read]).into_owned())
            .unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{body}"
        );
        stream.write_all(response.as_bytes()).unwrap();
    });
    (format!("http://{address}/v1beta/interactions?alt=sse"), rx)
}

fn adapter(endpoint: String) -> GeminiLlm {
    GeminiLlm::new(GeminiConfig::new(endpoint, "secret", "gemini-test")).unwrap()
}

#[test]
fn streams_text_and_usage_from_interactions_sse() {
    let body = concat!(
        "data: {\"event_type\":\"step.delta\",\"delta\":{\"type\":\"text\",\"text\":\"При\"}}\n\n",
        "data: {\"event_type\":\"step.delta\",\"delta\":{\"type\":\"text\",\"text\":\"вет\"}}\n\n",
        "data: {\"event_type\":\"interaction.completed\",\"interaction\":{\"status\":\"completed\",\"usage\":{\"total_input_tokens\":8,\"total_output_tokens\":3}}}\n\n",
        "data: [DONE]\n\n"
    );
    let provider = adapter(serve_once("200 OK", body));
    let mut sink = GeneratedTextBuffer::default();
    let usage = provider
        .stream(
            &LlmRequest {
                locale: "ru-RU".into(),
                instructions: None,
                user_input: "Ответь кратко".into(),
            },
            &Probe(AtomicBool::new(false)),
            &mut sink,
        )
        .unwrap();
    assert_eq!(sink.as_str(), "Привет");
    assert_eq!(usage.input_units, Some(8));
    assert_eq!(usage.input_unit, Some(UsageUnit::Token));
    assert_eq!(usage.output_units, Some(3));
    assert_eq!(usage.output_unit, Some(UsageUnit::Token));
}
#[test]
fn pull_stream_yields_text_before_completion_and_keeps_usage() {
    let body = concat!(
        "data: {\"event_type\":\"step.delta\",\"delta\":{\"type\":\"text\",\"text\":\"Первая.\"}}\n\n",
        "data: {\"event_type\":\"interaction.completed\",\"interaction\":{\"status\":\"completed\",\"usage\":{\"total_input_tokens\":5,\"total_output_tokens\":2}}}\n\n",
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
        Some("Первая.")
    );
    assert_eq!(stream.next_chunk(&probe).unwrap(), None);
    assert_eq!(stream.usage().input_units, Some(5));
    assert_eq!(stream.usage().output_units, Some(2));
}

#[test]
fn rejects_incomplete_stream_without_completed_and_done() {
    let body = "data: {\"event_type\":\"step.delta\",\"delta\":{\"type\":\"text\",\"text\":\"partial\"}}\n\n";
    let provider = adapter(serve_once("200 OK", body));
    let error = provider
        .stream(
            &LlmRequest {
                locale: "ru-RU".into(),
                instructions: None,
                user_input: "test".into(),
            },
            &Probe(AtomicBool::new(false)),
            &mut GeneratedTextBuffer::default(),
        )
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
    assert!(!error.retryable);
}

#[test]
fn maps_rate_limit_to_retryable_error() {
    let provider = adapter(serve_once("429 Too Many Requests", "rate limited"));
    let error = provider
        .stream(
            &LlmRequest {
                locale: "ru-RU".into(),
                instructions: None,
                user_input: "test".into(),
            },
            &Probe(AtomicBool::new(false)),
            &mut GeneratedTextBuffer::default(),
        )
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::RateLimited);
    assert!(error.retryable);
}

#[test]
fn rejects_external_plain_http() {
    let error = GeminiLlm::new(GeminiConfig::new(
        "http://example.com/v1beta/interactions?alt=sse",
        "secret",
        "gemini-test",
    ))
    .err()
    .expect("external plaintext endpoint must be rejected");
    assert_eq!(error.kind, ProviderErrorKind::PolicyDenied);
}

#[test]
fn pre_cancelled_request_never_starts_network_work() {
    let provider = adapter("http://127.0.0.1:1/v1beta/interactions?alt=sse".to_owned());
    let error = provider
        .stream(
            &LlmRequest {
                locale: "ru-RU".into(),
                instructions: None,
                user_input: "test".into(),
            },
            &Probe(AtomicBool::new(true)),
            &mut GeneratedTextBuffer::default(),
        )
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Cancelled);
}

#[test]
fn descriptor_does_not_expose_secret() {
    let config = GeminiConfig::new(
        "https://generativelanguage.googleapis.com/v1beta/interactions?alt=sse",
        "super-secret",
        "gemini-test",
    );
    let descriptor = config.descriptor();
    assert_eq!(descriptor.provider, "gemini");
    assert_eq!(descriptor.model, "gemini-test");
    assert_eq!(descriptor.representation, None);
}
