use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;

struct Probe(AtomicBool);

impl CancellationProbe for Probe {
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
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
    String::from_utf8_lossy(&request).into_owned()
}

fn serve(responses: Vec<(&'static str, String)>) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = tx.send(read_request(&mut stream));
            let headers = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(headers.as_bytes()).unwrap();
            stream.write_all(body.as_bytes()).unwrap();
        }
    });
    (format!("http://{address}"), rx)
}

fn adapter(endpoint: String) -> DidAgentStreamsAvatar {
    DidAgentStreamsAvatar::new(DidAgentStreamsConfig::new(
        endpoint,
        "secret-key",
        "agent-7",
    ))
    .unwrap()
}

fn session() -> RealtimeAvatarSession {
    RealtimeAvatarSession {
        provider_stream_id: "stream-1".to_owned(),
        provider_session_id: "session-1".to_owned(),
        offer: WebRtcSessionDescription {
            kind: "offer".to_owned(),
            sdp: "offer-sdp".to_owned(),
        },
        ice_servers: Vec::new(),
    }
}

#[test]
fn full_agents_streams_control_plane_matches_contract() {
    let create_body = r#"{"id":"stream-1","session_id":"session-1","offer":{"type":"offer","sdp":"offer-sdp"},"ice_servers":[{"urls":["stun:one","turn:two"],"username":"u","credential":"c"}],"fluent":true,"interrupt_enabled":true}"#;
    let (endpoint, captured) = serve(vec![
        ("201 Created", create_body.to_owned()),
        ("200 OK", "{}".to_owned()),
        ("200 OK", "{}".to_owned()),
        ("200 OK", "{}".to_owned()),
        ("200 OK", "{}".to_owned()),
        ("200 OK", "{}".to_owned()),
    ]);
    let provider = adapter(endpoint);
    let probe = Probe(AtomicBool::new(false));

    let live = provider.create_session(&probe).unwrap();
    assert_eq!(live.provider_stream_id, "stream-1");
    assert_eq!(live.provider_session_id, "session-1");
    assert_eq!(live.offer.kind, "offer");
    assert_eq!(live.ice_servers[0].urls.len(), 2);
    let control = provider.client_control(&live).unwrap();
    assert_eq!(control.data_channel_label, "JanusDataChannel");
    assert!(control.interrupt);

    let event = provider
        .parse_client_event(
            &live,
            r#"stream/started:{"metadata":{"videoId":"video-7"}}"#,
        )
        .unwrap();
    assert_eq!(
        event,
        Some(RealtimeAvatarClientEvent::PlaybackStarted {
            playback_id: "video-7".to_owned(),
        })
    );
    assert_eq!(
        provider
            .parse_client_event(&live, "stream/done:{}")
            .unwrap(),
        Some(RealtimeAvatarClientEvent::PlaybackDone)
    );
    assert_eq!(
        provider
            .parse_client_event(&live, r#"chat/partial:{"value":"ignored"}"#)
            .unwrap(),
        None
    );

    let command = provider
        .prepare_client_interrupt(&live, "video-7", &probe)
        .unwrap();
    assert_eq!(command.data_channel_label, "JanusDataChannel");
    let payload: serde_json::Value = serde_json::from_str(&command.payload).unwrap();
    assert_eq!(payload["type"], "stream/interrupt");
    assert_eq!(payload["videoId"], "video-7");
    assert!(payload["timestamp"].as_u64().is_some_and(|value| value > 0));
    assert!(!format!("{command:?}").contains("video-7"));

    provider
        .submit_answer(
            &live,
            &WebRtcSessionDescription {
                kind: "answer".to_owned(),
                sdp: "answer-sdp".to_owned(),
            },
            &probe,
        )
        .unwrap();
    provider
        .submit_ice_candidate(
            &live,
            &WebRtcIceCandidate {
                candidate: Some("candidate-x".to_owned()),
                sdp_mid: Some("0".to_owned()),
                sdp_mline_index: Some(0),
            },
            &probe,
        )
        .unwrap();
    provider.speak_text(&live, "Привет", &probe).unwrap();
    provider
        .speak_audio_url(&live, "https://cdn.example.com/voice.mp3", &probe)
        .unwrap();
    provider.close_session(&live).unwrap();
    assert!(provider.client_control(&live).is_none());

    let requests: Vec<String> = (0..6).map(|_| captured.recv().unwrap()).collect();
    let lower = requests
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    assert!(
        lower
            .iter()
            .all(|request| request.contains("authorization: basic secret-key"))
    );
    assert!(requests[0].starts_with("POST /agents/agent-7/streams "));
    assert!(requests[0].contains("\"fluent\":true"));
    assert!(requests[1].starts_with("POST /agents/agent-7/streams/stream-1/sdp "));
    assert!(requests[1].contains("\"session_id\":\"session-1\""));
    assert!(requests[1].contains("\"type\":\"answer\""));
    assert!(requests[2].starts_with("POST /agents/agent-7/streams/stream-1/ice "));
    assert!(requests[2].contains("\"candidate\":\"candidate-x\""));
    assert!(requests[3].starts_with("POST /agents/agent-7/streams/stream-1 "));
    assert!(requests[3].contains("\"type\":\"text\""));
    assert!(requests[3].contains("Привет"));
    assert!(requests[4].starts_with("POST /agents/agent-7/streams/stream-1 "));
    assert!(requests[4].contains("\"type\":\"audio\""));
    assert!(requests[4].contains("\"audio_url\":\"https://cdn.example.com/voice.mp3\""));
    assert!(requests[5].starts_with("DELETE /agents/agent-7/streams/stream-1 "));
}

#[test]
fn legacy_or_non_interruptible_stream_never_advertises_client_interrupt() {
    for create_body in [
        r#"{"id":"stream-1","session_id":"session-1","offer":{"type":"offer","sdp":"offer-sdp"},"fluent":false,"interrupt_enabled":true}"#,
        r#"{"id":"stream-1","session_id":"session-1","offer":{"type":"offer","sdp":"offer-sdp"},"fluent":true,"interrupt_enabled":false}"#,
    ] {
        let (endpoint, _) = serve(vec![("201 Created", create_body.to_owned())]);
        let provider = adapter(endpoint);
        let live = provider
            .create_session(&Probe(AtomicBool::new(false)))
            .unwrap();
        assert!(provider.client_control(&live).is_none());
        let error = provider
            .prepare_client_interrupt(&live, "video-7", &Probe(AtomicBool::new(false)))
            .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::Unavailable);
    }
}

#[test]
fn malformed_client_playback_event_fails_closed() {
    let create_body = r#"{"id":"stream-1","session_id":"session-1","offer":{"type":"offer","sdp":"offer-sdp"},"fluent":true,"interrupt_enabled":true}"#;
    let (endpoint, _) = serve(vec![("201 Created", create_body.to_owned())]);
    let provider = adapter(endpoint);
    let live = provider
        .create_session(&Probe(AtomicBool::new(false)))
        .unwrap();

    for message in [
        "stream/started:{}",
        r#"stream/started:{"metadata":{}}"#,
        "stream/started:not-json",
    ] {
        let error = provider.parse_client_event(&live, message).unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
    }
}

#[test]
fn pre_cancelled_session_creation_never_starts_network_work() {
    let provider = adapter("http://127.0.0.1:1".to_owned());
    let error = provider
        .create_session(&Probe(AtomicBool::new(true)))
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Cancelled);
}

#[test]
fn empty_speak_text_is_rejected_before_network() {
    let provider = adapter("http://127.0.0.1:1".to_owned());
    let error = provider
        .speak_text(&session(), "   ", &Probe(AtomicBool::new(false)))
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
}

#[test]
fn maps_rate_limit_to_retryable_provider_error() {
    let (endpoint, _) = serve(vec![("429 Too Many Requests", "{}".to_owned())]);
    let error = adapter(endpoint)
        .create_session(&Probe(AtomicBool::new(false)))
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::RateLimited);
    assert!(error.retryable);
}

#[test]
fn rejects_external_plain_http() {
    let error = DidAgentStreamsAvatar::new(DidAgentStreamsConfig::new(
        "http://example.com",
        "secret",
        "agent",
    ))
    .err()
    .expect("external plaintext must fail");
    assert_eq!(error.kind, ProviderErrorKind::PolicyDenied);
}

#[test]
fn malformed_create_session_response_is_rejected() {
    let (endpoint, _) = serve(vec![("201 Created", "{}".to_owned())]);
    let error = adapter(endpoint)
        .create_session(&Probe(AtomicBool::new(false)))
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
}

#[test]
fn descriptor_and_capabilities_do_not_expose_secret() {
    let provider = DidAgentStreamsAvatar::new(DidAgentStreamsConfig::new(
        "https://api.d-id.com",
        "super-secret",
        "agent-x",
    ))
    .unwrap();
    let descriptor = provider.descriptor();
    assert_eq!(descriptor.provider, "d-id-agents-streams");
    assert_eq!(descriptor.representation.as_deref(), Some("agent-x"));
    let capabilities = provider.capabilities();
    assert!(capabilities.supports(RealtimeAvatarCapability::TextInput));
    assert!(capabilities.supports(RealtimeAvatarCapability::AudioUrlInput));
    assert!(!capabilities.supports(RealtimeAvatarCapability::Interrupt));
    assert!(!format!("{descriptor:?}").contains("super-secret"));
}

#[test]
fn insecure_audio_url_is_rejected_before_network() {
    let provider = adapter("http://127.0.0.1:1".to_owned());
    let error = provider
        .speak_audio_url(
            &session(),
            "http://example.com/private.wav",
            &Probe(AtomicBool::new(false)),
        )
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::PolicyDenied);
}

#[test]
fn path_identifiers_are_percent_encoded_instead_of_becoming_routes() {
    let (endpoint, captured) = serve(vec![("200 OK", "{}".to_owned())]);
    let provider = DidAgentStreamsAvatar::new(DidAgentStreamsConfig::new(
        endpoint,
        "secret-key",
        "agent/7?admin=true",
    ))
    .unwrap();
    let mut live = session();
    live.provider_stream_id = "stream/1?delete=true".to_owned();

    provider
        .speak_text(&live, "Привет", &Probe(AtomicBool::new(false)))
        .unwrap();
    let request = captured.recv().unwrap();
    assert!(
        request
            .starts_with("POST /agents/agent%2F7%3Fadmin=true/streams/stream%2F1%3Fdelete=true ")
    );
}

#[test]
fn invalid_session_is_rejected_before_network() {
    let provider = adapter("http://127.0.0.1:1".to_owned());
    let mut invalid = session();
    invalid.provider_stream_id.clear();
    let error = provider
        .speak_text(&invalid, "Привет", &Probe(AtomicBool::new(false)))
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
}

#[test]
fn endpoint_userinfo_is_rejected() {
    let error = DidAgentStreamsAvatar::new(DidAgentStreamsConfig::new(
        "https://embedded:secret@example.com",
        "secret",
        "agent",
    ))
    .err()
    .expect("endpoint userinfo must fail closed");
    assert_eq!(error.kind, ProviderErrorKind::PolicyDenied);
}

#[test]
fn audio_url_userinfo_is_rejected_before_network() {
    let provider = adapter("http://127.0.0.1:1".to_owned());
    let error = provider
        .speak_audio_url(
            &session(),
            "https://embedded:secret@example.com/voice.wav",
            &Probe(AtomicBool::new(false)),
        )
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::PolicyDenied);
}
