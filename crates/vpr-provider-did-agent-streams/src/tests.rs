use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use vpr_integration::ProviderErrorKind;

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
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            total = Some(header_end + 4 + content_length);
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

fn fluent_adapter(endpoint: String) -> DidAgentStreamsAvatar {
    DidAgentStreamsAvatar::new(
        DidAgentStreamsConfig::new(endpoint, "secret-key", "agent-7").with_fluent(true),
    )
    .unwrap()
}

fn legacy_agent_body() -> String {
    r#"{"presenter":{"type":"clip"}}"#.to_owned()
}

fn expressive_agent_body() -> String {
    r#"{"presenter":{"type":"expressive"}}"#.to_owned()
}

#[test]
fn presenter_probe_is_read_only_and_uses_authorization() {
    let (endpoint, captured) = serve(vec![("200 OK", expressive_agent_body())]);
    let provider = adapter(endpoint);

    assert_eq!(provider.probe_presenter_type().unwrap(), "expressive");
    let request = captured.recv().unwrap();
    assert!(request.starts_with("GET /agents/agent-7 "));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: basic secret-key")
    );
}

#[test]
fn presenter_probe_reports_provider_policy_denial_without_creating_session() {
    let (endpoint, captured) = serve(vec![("401 Unauthorized", "{}".to_owned())]);
    let provider = adapter(endpoint);

    let error = provider.probe_presenter_type().unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::PolicyDenied);
    assert!(captured.recv().unwrap().starts_with("GET /agents/agent-7 "));
}

fn session() -> RealtimeAvatarSession {
    RealtimeAvatarSession {
        provider_resource_id: "stream-1".to_owned(),
        provider_session_id: "session-1".to_owned(),
        transport: RealtimeAvatarTransport::WebRtc {
            offer: WebRtcSessionDescription {
                kind: "offer".to_owned(),
                sdp: "offer-sdp".to_owned(),
            },
            ice_servers: Vec::new(),
        },
    }
}

fn assert_client_interrupt_contract(
    provider: &DidAgentStreamsAvatar,
    live: &RealtimeAvatarSession,
    probe: &Probe,
) {
    let control = provider.client_control(live).unwrap();
    assert_eq!(
        control.event_route,
        Some(RealtimeAvatarClientRoute::WebRtcDataChannel {
            label: "JanusDataChannel".to_owned(),
        })
    );
    assert!(control.interrupt);
    assert!(control.interrupt_requires_playback_id);
    assert!(!control.text_input);

    let event = provider
        .parse_client_event(live, r#"stream/started:{"metadata":{"videoId":"video-7"}}"#)
        .unwrap();
    assert_eq!(
        event,
        Some(RealtimeAvatarClientEvent::PlaybackStarted {
            playback_id: "video-7".to_owned(),
        })
    );
    assert_eq!(
        provider.parse_client_event(live, "stream/done:{}").unwrap(),
        Some(RealtimeAvatarClientEvent::PlaybackDone)
    );
    assert_eq!(
        provider
            .parse_client_event(live, r#"chat/partial:{"value":"ignored"}"#)
            .unwrap(),
        None
    );

    let command = provider
        .prepare_client_interrupt(live, Some("video-7"), probe)
        .unwrap();
    assert_eq!(
        command.route,
        RealtimeAvatarClientRoute::WebRtcDataChannel {
            label: "JanusDataChannel".to_owned(),
        }
    );
    let payload: serde_json::Value = serde_json::from_str(&command.payload).unwrap();
    assert_eq!(payload["type"], "stream/interrupt");
    assert_eq!(payload["videoId"], "video-7");
    assert!(payload["timestamp"].as_u64().is_some_and(|value| value > 0));
    assert!(!format!("{command:?}").contains("video-7"));
}

#[test]
fn full_agents_streams_control_plane_matches_contract() {
    let create_body = r#"{"id":"stream-1","session_id":"session-1","offer":{"type":"offer","sdp":"offer-sdp"},"ice_servers":[{"urls":["stun:one","turn:two"],"username":"u","credential":"c"}],"fluent":true,"interrupt_enabled":true}"#;
    let (endpoint, captured) = serve(vec![
        ("200 OK", legacy_agent_body()),
        ("201 Created", create_body.to_owned()),
        ("200 OK", "{}".to_owned()),
        ("200 OK", "{}".to_owned()),
        ("200 OK", "{}".to_owned()),
        ("200 OK", "{}".to_owned()),
        ("200 OK", "{}".to_owned()),
    ]);
    let provider = fluent_adapter(endpoint);
    let probe = Probe(AtomicBool::new(false));

    let live = provider.create_session(&probe).unwrap();
    assert_eq!(live.provider_resource_id, "stream-1");
    assert_eq!(live.provider_session_id, "session-1");
    let RealtimeAvatarTransport::WebRtc { offer, ice_servers } = &live.transport else {
        panic!("expected WebRTC transport");
    };
    assert_eq!(offer.kind, "offer");
    assert_eq!(ice_servers[0].urls.len(), 2);
    assert_client_interrupt_contract(&provider, &live, &probe);

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

    let requests: Vec<String> = (0..7).map(|_| captured.recv().unwrap()).collect();
    let lower = requests
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    assert!(
        lower
            .iter()
            .all(|request| request.contains("authorization: basic secret-key"))
    );
    assert!(requests[0].starts_with("GET /agents/agent-7 "));
    assert!(requests[1].starts_with("POST /agents/agent-7/streams "));
    assert!(requests[1].contains("\"fluent\":true"));
    assert!(requests[2].starts_with("POST /agents/agent-7/streams/stream-1/sdp "));
    assert!(requests[2].contains("\"session_id\":\"session-1\""));
    assert!(requests[2].contains("\"type\":\"answer\""));
    assert!(requests[3].starts_with("POST /agents/agent-7/streams/stream-1/ice "));
    assert!(requests[3].contains("\"candidate\":\"candidate-x\""));
    assert!(requests[4].starts_with("POST /agents/agent-7/streams/stream-1 "));
    assert!(requests[4].contains("\"type\":\"text\""));
    assert!(requests[4].contains("Привет"));
    assert!(requests[5].starts_with("POST /agents/agent-7/streams/stream-1 "));
    assert!(requests[5].contains("\"type\":\"audio\""));
    assert!(requests[5].contains("\"audio_url\":\"https://cdn.example.com/voice.mp3\""));
    assert!(requests[6].starts_with("DELETE /agents/agent-7/streams/stream-1 "));
}

#[test]
fn legacy_or_non_interruptible_stream_never_advertises_client_interrupt() {
    for create_body in [
        r#"{"id":"stream-1","session_id":"session-1","offer":{"type":"offer","sdp":"offer-sdp"},"fluent":false,"interrupt_enabled":true}"#,
        r#"{"id":"stream-1","session_id":"session-1","offer":{"type":"offer","sdp":"offer-sdp"},"fluent":true,"interrupt_enabled":false}"#,
    ] {
        let (endpoint, _) = serve(vec![
            ("200 OK", legacy_agent_body()),
            ("201 Created", create_body.to_owned()),
        ]);
        let provider = adapter(endpoint);
        let live = provider
            .create_session(&Probe(AtomicBool::new(false)))
            .unwrap();
        assert!(provider.client_control(&live).is_none());
        let event_error = provider
            .parse_client_event(
                &live,
                r#"stream/started:{"metadata":{"videoId":"video-7"}}"#,
            )
            .unwrap_err();
        assert_eq!(event_error.kind, ProviderErrorKind::Unavailable);
        let error = provider
            .prepare_client_interrupt(&live, Some("video-7"), &Probe(AtomicBool::new(false)))
            .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::Unavailable);
    }
}

#[test]
fn malformed_client_playback_event_fails_closed() {
    let create_body = r#"{"id":"stream-1","session_id":"session-1","offer":{"type":"offer","sdp":"offer-sdp"},"fluent":true,"interrupt_enabled":true}"#;
    let (endpoint, _) = serve(vec![
        ("200 OK", legacy_agent_body()),
        ("201 Created", create_body.to_owned()),
    ]);
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
fn forbidden_presenter_metadata_falls_back_to_historical_stream_path() {
    let create_body = r#"{"id":"stream-1","session_id":"session-1","offer":{"type":"offer","sdp":"offer-sdp"},"fluent":false,"interrupt_enabled":false}"#;
    let (endpoint, captured) = serve(vec![
        ("403 Forbidden", "{}".to_owned()),
        ("201 Created", create_body.to_owned()),
    ]);
    let provider = adapter(endpoint);
    let live = provider
        .create_session(&Probe(AtomicBool::new(false)))
        .unwrap();

    assert!(matches!(
        live.transport,
        RealtimeAvatarTransport::WebRtc { .. }
    ));
    let requests: Vec<String> = (0..2).map(|_| captured.recv().unwrap()).collect();
    assert!(requests[0].starts_with("GET /agents/agent-7 "));
    assert!(requests[1].starts_with("POST /agents/agent-7/streams "));
}

#[test]
fn runtime_access_probe_uses_legacy_stream_only_when_metadata_is_forbidden() {
    let create_body = r#"{"id":"stream-1","session_id":"session-1","offer":{"type":"offer","sdp":"offer-sdp"},"fluent":false,"interrupt_enabled":false}"#;
    let (endpoint, captured) = serve(vec![
        ("403 Forbidden", "{}".to_owned()),
        ("201 Created", create_body.to_owned()),
        ("200 OK", "{}".to_owned()),
    ]);
    let provider = adapter(endpoint);

    assert_eq!(
        provider.probe_runtime_access().unwrap(),
        DidRuntimeAccessProbe::LegacyStreamFallback
    );
    let requests: Vec<String> = (0..3).map(|_| captured.recv().unwrap()).collect();
    assert!(requests[0].starts_with("GET /agents/agent-7 "));
    assert!(requests[1].starts_with("POST /agents/agent-7/streams "));
    assert!(requests[2].starts_with("DELETE /agents/agent-7/streams/stream-1 "));
}

#[test]
fn unauthorized_presenter_metadata_does_not_fall_back_to_stream_creation() {
    let (endpoint, captured) = serve(vec![("401 Unauthorized", "{}".to_owned())]);
    let provider = adapter(endpoint);

    let error = provider
        .create_session(&Probe(AtomicBool::new(false)))
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::PolicyDenied);
    assert!(captured.recv().unwrap().starts_with("GET /agents/agent-7 "));
    assert!(captured.try_recv().is_err());
}

#[test]
fn account_auth_probe_uses_account_credits_and_basic_authorization() {
    let (endpoint, captured) = serve(vec![("200 OK", "[]".to_owned())]);
    let provider = adapter(endpoint);

    provider.probe_account_auth_detailed().unwrap();

    let request = captured.recv().unwrap();
    assert!(request.starts_with("GET /credits "));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: basic secret-key")
    );
}

#[test]
fn account_auth_probe_fails_on_explicit_zero_balance() {
    let (endpoint, captured) = serve(vec![(
        "200 OK",
        r#"{"remaining":0.0,"total":15.0}"#.to_owned(),
    )]);
    let provider = adapter(endpoint);

    let error = provider.probe_account_auth_detailed().unwrap_err();
    let DidRuntimeAccessFailure::Provider(error) = error else {
        panic!("expected typed provider error");
    };
    assert_eq!(error.kind, ProviderErrorKind::InsufficientCredits);
    assert!(!error.retryable);
    assert!(captured.recv().unwrap().starts_with("GET /credits "));
}

#[test]
fn account_auth_probe_fails_when_all_legacy_credit_buckets_are_empty() {
    let (endpoint, _) = serve(vec![(
        "200 OK",
        r#"[{"remaining":0.0,"total":5.0},{"remaining":0.0,"total":10.0}]"#.to_owned(),
    )]);
    let provider = adapter(endpoint);

    let error = provider.probe_account_auth_detailed().unwrap_err();
    let DidRuntimeAccessFailure::Provider(error) = error else {
        panic!("expected typed provider error");
    };
    assert_eq!(error.kind, ProviderErrorKind::InsufficientCredits);
}

#[test]
fn account_auth_probe_accepts_positive_known_balance() {
    let (endpoint, _) = serve(vec![(
        "200 OK",
        r#"{"remaining":1.25,"total":15.0}"#.to_owned(),
    )]);
    let provider = adapter(endpoint);

    provider.probe_account_auth_detailed().unwrap();
}

#[test]
fn account_auth_probe_does_not_treat_not_found_as_zero_balance() {
    let (endpoint, _) = serve(vec![("404 Not Found", "{}".to_owned())]);
    let provider = adapter(endpoint);

    let error = provider.probe_account_auth_detailed().unwrap_err();
    let DidRuntimeAccessFailure::Provider(error) = error else {
        panic!("expected typed provider error");
    };
    assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
}

#[test]
fn account_auth_probe_preserves_unauthorized_status() {
    let (endpoint, captured) = serve(vec![("401 Unauthorized", "{}".to_owned())]);
    let provider = adapter(endpoint);

    assert_eq!(
        provider.probe_account_auth_detailed().unwrap_err(),
        DidRuntimeAccessFailure::Unauthorized
    );
    assert!(captured.recv().unwrap().starts_with("GET /credits "));
}

#[test]
fn detailed_probe_reports_401_without_stream_fallback() {
    let (endpoint, captured) = serve(vec![("401 Unauthorized", "{}".to_owned())]);
    let provider = adapter(endpoint);

    assert_eq!(
        provider.probe_runtime_access_detailed().unwrap_err(),
        DidRuntimeAccessFailure::Unauthorized
    );
    assert!(captured.recv().unwrap().starts_with("GET /agents/agent-7 "));
    assert!(captured.try_recv().is_err());
}

#[test]
fn detailed_probe_reports_403_when_legacy_stream_is_forbidden() {
    let (endpoint, captured) = serve(vec![
        ("403 Forbidden", "{}".to_owned()),
        ("403 Forbidden", "{}".to_owned()),
    ]);
    let provider = adapter(endpoint);

    assert_eq!(
        provider.probe_runtime_access_detailed().unwrap_err(),
        DidRuntimeAccessFailure::Forbidden
    );
    let requests: Vec<String> = (0..2).map(|_| captured.recv().unwrap()).collect();
    assert!(requests[0].starts_with("GET /agents/agent-7 "));
    assert!(requests[1].starts_with("POST /agents/agent-7/streams "));
}

#[test]
fn expressive_agent_negotiates_livekit_without_leaking_credentials() {
    let (endpoint, captured) = serve(vec![
        ("200 OK", expressive_agent_body()),
        (
            "201 Created",
            r#"{"id":"live-session-1","session_url":"wss://livekit.example.test","session_token":"private-livekit-token"}"#.to_owned(),
        ),
    ]);
    let provider = adapter(endpoint);
    let probe = Probe(AtomicBool::new(false));

    let live = provider.create_session(&probe).unwrap();
    assert_eq!(live.provider_resource_id, "live-session-1");
    let RealtimeAvatarTransport::LiveKit { server_url, token } = &live.transport else {
        panic!("expected LiveKit transport");
    };
    assert_eq!(server_url, "wss://livekit.example.test");
    assert_eq!(token, "private-livekit-token");

    let control = provider.client_control(&live).unwrap();
    assert!(control.interrupt);
    assert!(!control.interrupt_requires_playback_id);
    assert!(control.text_input);

    assert_eq!(
        provider
            .parse_client_event(&live, r#"{"subject":"stream-video/done"}"#)
            .unwrap(),
        Some(RealtimeAvatarClientEvent::VideoGenerationDone)
    );
    // Generation completion is not a provider-confirmed audible playback completion.
    for (subject, expected) in [
        (
            "stream-video/started",
            RealtimeAvatarClientEvent::VideoGenerationStarted,
        ),
        (
            "stream-video/error",
            RealtimeAvatarClientEvent::VideoGenerationFailed,
        ),
        ("chat/answer", RealtimeAvatarClientEvent::Informational),
        ("chat/partial", RealtimeAvatarClientEvent::Informational),
        ("tool-call/done", RealtimeAvatarClientEvent::Informational),
    ] {
        let message = serde_json::json!({"subject":subject,"content":"untrusted"}).to_string();
        assert_eq!(
            provider.parse_client_event(&live, &message).unwrap(),
            Some(expected)
        );
    }
    // Unknown provider subjects are classified without returning any raw subject
    // or the untrusted content that may contain private owner/provider data.
    for (subject, expected) in [
        (
            "chat/new-event",
            RealtimeAvatarClientEvent::UnknownChatEvent,
        ),
        (
            "stream-video/new-event",
            RealtimeAvatarClientEvent::UnknownVideoEvent,
        ),
        (
            "tool-call/new-event",
            RealtimeAvatarClientEvent::UnknownToolEvent,
        ),
        (
            "unknown/event",
            RealtimeAvatarClientEvent::UnknownOtherEvent,
        ),
    ] {
        let message = serde_json::json!({"subject":subject,"content":"PRIVATE_UNTRUSTED_PAYLOAD"})
            .to_string();
        let classified = provider.parse_client_event(&live, &message).unwrap();
        assert_eq!(classified, Some(expected));
        assert!(!format!("{classified:?}").contains("PRIVATE_UNTRUSTED_PAYLOAD"));
        assert!(!format!("{classified:?}").contains(subject));
    }

    let text = provider
        .prepare_client_text(&live, "Привет", &probe)
        .unwrap();
    assert_eq!(
        text.route,
        RealtimeAvatarClientRoute::LiveKitTextTopic {
            topic: "did.speak".to_owned(),
        }
    );
    let text_payload: serde_json::Value = serde_json::from_str(&text.payload).unwrap();
    assert_eq!(text_payload["script"]["type"], "text");
    assert_eq!(text_payload["script"]["input"], "Привет");
    assert_eq!(text_payload["script"]["should_queue_speaks"], true);

    let interrupt = provider
        .prepare_client_interrupt(&live, None, &probe)
        .unwrap();
    assert_eq!(
        interrupt.route,
        RealtimeAvatarClientRoute::LiveKitTextTopic {
            topic: "did.interrupt".to_owned(),
        }
    );

    provider.close_session(&live).unwrap();
    let requests: Vec<String> = (0..2).map(|_| captured.recv().unwrap()).collect();
    assert!(requests[0].starts_with("GET /agents/agent-7 "));
    assert!(requests[1].starts_with("POST /v2/agents/agent-7/sessions "));
    let debug = format!("{live:?} {text:?} {interrupt:?}");
    assert!(!debug.contains("private-livekit-token"));
    assert!(!debug.contains("Привет"));
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
fn maps_insufficient_credits_without_collapsing_to_internal_error() {
    let (endpoint, _) = serve(vec![
        ("200 OK", expressive_agent_body()),
        (
            "402 Payment Required",
            r#"{"kind":"InsufficientCreditsError"}"#.to_owned(),
        ),
    ]);
    let error = adapter(endpoint)
        .create_session(&Probe(AtomicBool::new(false)))
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::InsufficientCredits);
    assert!(!error.retryable);
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
    let (endpoint, _) = serve(vec![
        ("200 OK", legacy_agent_body()),
        ("201 Created", "{}".to_owned()),
    ]);
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
    live.provider_resource_id = "stream/1?delete=true".to_owned();

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
    invalid.provider_resource_id.clear();
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

#[derive(Default)]
struct MockEchoBackend {
    calls: std::sync::Mutex<Vec<String>>,
}

impl DidEchoBackend for MockEchoBackend {
    fn open(
        &self,
        session_id: &str,
        session_url: &str,
        echo_token: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        assert!(!cancellation.is_cancelled());
        assert_eq!(session_id, "echo-session-1");
        assert_eq!(session_url, "wss://livekit.example.test/room/agent-echo");
        assert_eq!(echo_token, "server-ONLY-echo-token");
        self.calls.lock().unwrap().push("open".to_owned());
        Ok(())
    }

    fn speak(
        &self,
        session_id: &str,
        text: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        assert!(!cancellation.is_cancelled());
        assert_eq!(session_id, "echo-session-1");
        assert_eq!(text, "Серверный ответ.");
        self.calls.lock().unwrap().push("speak".to_owned());
        Ok(())
    }

    fn stop(&self, session_id: &str) -> Result<(), ProviderError> {
        assert_eq!(session_id, "echo-session-1");
        self.calls.lock().unwrap().push("stop".to_owned());
        Ok(())
    }
}

#[test]
fn echo_sender_stays_server_private_and_browser_speak_is_impossible() {
    use std::sync::Arc;

    let body = r#"{"id":"echo-session-1","session_url":"wss://livekit.example.test/room/agent-echo","session_token":"viewer-token","echo_token":"server-ONLY-echo-token"}"#;
    let (endpoint, captured) = serve(vec![
        ("200 OK", expressive_agent_body()),
        ("201 Created", body.to_owned()),
    ]);
    let backend = Arc::new(MockEchoBackend::default());
    let provider = adapter(endpoint).with_echo_backend(backend.clone());
    let probe = Probe(AtomicBool::new(false));
    let session = provider.create_session(&probe).unwrap();
    let create = captured.recv().unwrap();
    assert!(create.starts_with("GET /agents/agent-7 "));
    let created = captured.recv().unwrap();
    assert!(created.starts_with("POST /v2/agents/agent-7/sessions "));
    assert!(created.contains(r#""session_type":"echo""#));

    if let RealtimeAvatarTransport::LiveKit { server_url, token } = &session.transport {
        assert_eq!(token, "viewer-token");
        // Vendor Echo includes /room/<id>, but LiveKit Room.connect expects
        // the websocket server URL; the room is authorized by the token.
        assert_eq!(server_url, "wss://livekit.example.test");
    } else {
        panic!("Echo must use the LiveKit viewer transport");
    }
    assert!(!format!("{session:?}").contains("server-ONLY-echo-token"));
    let control = provider.client_control(&session).unwrap();
    assert!(
        !control.text_input,
        "Echo viewer must not publish did.speak"
    );
    assert!(!control.interrupt);
    assert!(
        provider
            .prepare_client_text(&session, "forbidden browser text", &probe)
            .is_err()
    );
    assert!(
        provider
            .prepare_client_interrupt(&session, None, &probe)
            .is_err()
    );
    provider
        .speak_text(&session, "Серверный ответ.", &probe)
        .unwrap();
    provider.close_session(&session).unwrap();
    assert!(
        provider
            .speak_text(&session, "stale after revoke", &probe)
            .is_err()
    );
    assert!(!provider.client_control(&session).unwrap().text_input);
    assert_eq!(
        *backend.calls.lock().unwrap(),
        vec!["open", "speak", "stop"]
    );
}

#[test]
fn duplicate_echo_session_id_never_opens_a_second_sender_or_replaces_the_first() {
    use std::sync::Arc;

    let body = r#"{"id":"echo-session-1","session_url":"wss://livekit.example.test/room/agent-echo","session_token":"viewer-token","echo_token":"server-ONLY-echo-token"}"#;
    let (endpoint, _captured) = serve(vec![
        ("200 OK", expressive_agent_body()),
        ("201 Created", body.to_owned()),
        ("200 OK", expressive_agent_body()),
        ("201 Created", body.to_owned()),
    ]);
    let backend = Arc::new(MockEchoBackend::default());
    let provider = adapter(endpoint).with_echo_backend(backend.clone());
    let probe = Probe(AtomicBool::new(false));
    let first = provider.create_session(&probe).unwrap();
    let duplicate = provider.create_session(&probe).unwrap_err();
    assert_eq!(duplicate.kind, ProviderErrorKind::InvalidResponse);
    assert_eq!(*backend.calls.lock().unwrap(), vec!["open"]);
    assert!(
        provider
            .echo_sessions
            .contains(&first.provider_session_id)
            .unwrap()
    );

    provider
        .speak_text(&first, "Серверный ответ.", &probe)
        .unwrap();
    provider.close_session(&first).unwrap();
    assert_eq!(
        *backend.calls.lock().unwrap(),
        vec!["open", "speak", "stop"]
    );
}

struct CancelledDuringEchoOpen {
    cancellation: std::sync::Arc<Probe>,
    calls: std::sync::Mutex<Vec<&'static str>>,
}

impl DidEchoBackend for CancelledDuringEchoOpen {
    fn open(
        &self,
        _session_id: &str,
        _session_url: &str,
        _echo_token: &str,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        self.calls.lock().unwrap().push("open");
        // A second tab can revoke while the private LiveKit room connects.
        self.cancellation.0.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn speak(
        &self,
        _session_id: &str,
        _text: &str,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        panic!("cancelled Echo session must never send speech");
    }

    fn stop(&self, _session_id: &str) -> Result<(), ProviderError> {
        self.calls.lock().unwrap().push("stop");
        Ok(())
    }
}

#[test]
fn echo_start_cancelled_during_open_stops_private_sender_and_returns_no_session() {
    use std::sync::Arc;

    let body = r#"{"id":"echo-session-1","session_url":"wss://livekit.example.test/room/agent-echo","session_token":"viewer-token","echo_token":"server-ONLY-echo-token"}"#;
    let (endpoint, _captured) = serve(vec![
        ("200 OK", expressive_agent_body()),
        ("201 Created", body.to_owned()),
    ]);
    let cancellation = Arc::new(Probe(AtomicBool::new(false)));
    let backend = Arc::new(CancelledDuringEchoOpen {
        cancellation: Arc::clone(&cancellation),
        calls: std::sync::Mutex::new(Vec::new()),
    });
    let provider = adapter(endpoint).with_echo_backend(backend.clone());
    let error = provider.create_session(cancellation.as_ref()).unwrap_err();

    assert_eq!(error.kind, ProviderErrorKind::Cancelled);
    assert_eq!(*backend.calls.lock().unwrap(), vec!["open", "stop"]);
    assert!(!provider.echo_sessions.contains("echo-session-1").unwrap());
}

#[test]
fn echo_requires_a_real_sender_token_and_never_falls_back_to_browser_speak() {
    use std::sync::Arc;

    let (endpoint, captured) = serve(vec![
        ("200 OK", expressive_agent_body()),
        ("201 Created", r#"{"id":"echo-session-1","session_url":"wss://livekit.example.test/room/agent-echo","session_token":"viewer-token"}"#.to_owned()),
    ]);
    let backend = Arc::new(MockEchoBackend::default());
    let provider = adapter(endpoint).with_echo_backend(backend.clone());
    let error = provider
        .create_session(&Probe(AtomicBool::new(false)))
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
    assert!(backend.calls.lock().unwrap().is_empty());
    assert!(captured.recv().unwrap().starts_with("GET /agents/agent-7 "));
    assert!(
        captured
            .recv()
            .unwrap()
            .contains(r#""session_type":"echo""#)
    );
}
