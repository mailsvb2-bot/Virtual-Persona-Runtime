use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
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
    format!("http://{address}/v1/messages")
}
fn adapter(endpoint: String) -> AnthropicLlm {
    AnthropicLlm::new(AnthropicConfig::new(endpoint, "secret", "claude-test")).unwrap()
}

#[test]
fn streams_text_and_usage_from_messages_sse() {
    let body = concat!(
        "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":9,\"output_tokens\":1}}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"При\"}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"вет\"}}\n\n",
        "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":4}}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    );
    let provider = adapter(serve_once("200 OK", body));
    let probe = Probe(AtomicBool::new(false));
    let mut sink = GeneratedTextBuffer::default();
    let usage = provider
        .stream(
            &LlmRequest {
                locale: "ru-RU".into(),
                context: "Ответь кратко".into(),
            },
            &probe,
            &mut sink,
        )
        .unwrap();
    assert_eq!(sink.as_str(), "Привет");
    assert_eq!(usage.input_units, Some(9));
    assert_eq!(usage.output_units, Some(4));
}

#[test]
fn rejects_truncated_stream_without_message_stop() {
    let body = concat!(
        "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"partial\"}}\n\n"
    );
    let provider = adapter(serve_once("200 OK", body));
    let error = provider
        .stream(
            &LlmRequest {
                locale: "ru-RU".into(),
                context: "test".into(),
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
                context: "test".into(),
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
    let error = AnthropicLlm::new(AnthropicConfig::new(
        "http://example.com/v1/messages",
        "secret",
        "claude-test",
    ))
    .err()
    .expect("external plaintext endpoint must be rejected");
    assert_eq!(error.kind, ProviderErrorKind::PolicyDenied);
}
#[test]
fn pre_cancelled_request_never_starts_network_work() {
    let provider = adapter("http://127.0.0.1:1/v1/messages".to_owned());
    let error = provider
        .stream(
            &LlmRequest {
                locale: "ru-RU".into(),
                context: "test".into(),
            },
            &Probe(AtomicBool::new(true)),
            &mut GeneratedTextBuffer::default(),
        )
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Cancelled);
}

#[test]
fn descriptor_does_not_expose_secret() {
    let config = AnthropicConfig::new(
        "https://api.anthropic.com/v1/messages",
        "super-secret",
        "claude-test",
    );
    let descriptor = config.descriptor();
    assert_eq!(descriptor.provider, "anthropic");
    assert_eq!(descriptor.model, "claude-test");
    assert_eq!(descriptor.representation, None);
}
