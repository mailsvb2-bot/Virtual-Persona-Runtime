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

fn provider(endpoint: String) -> LocalOpenSourceAvatar {
    LocalOpenSourceAvatar::new(LocalOpenSourceAvatarConfig::new(endpoint, "worker-secret")).unwrap()
}

#[test]
fn full_worker_control_plane_matches_contract() {
    let create = r#"{"id":"local-1","offer":{"type":"offer","sdp":"offer-sdp"},"ice_servers":[{"urls":"stun:example.test"}]}"#;
    let (endpoint, captured) = serve(vec![
        ("201 Created", create.to_owned()),
        ("200 OK", "{}".to_owned()),
        ("200 OK", "{}".to_owned()),
        ("200 OK", "{}".to_owned()),
        ("200 OK", "{}".to_owned()),
        ("200 OK", r#"{"session_id":"local-1","audio_stopped":true,"video_stopped":true}"#.to_owned()),
    ]);
    let avatar = provider(endpoint);
    let probe = Probe(AtomicBool::new(false));
    let session = avatar.create_session(&probe).unwrap();
    avatar
        .submit_answer(
            &session,
            &WebRtcSessionDescription {
                kind: "answer".to_owned(),
                sdp: "answer-sdp".to_owned(),
            },
            &probe,
        )
        .unwrap();
    avatar
        .submit_ice_candidate(
            &session,
            &WebRtcIceCandidate {
                candidate: Some("candidate-x".to_owned()),
                sdp_mid: Some("0".to_owned()),
                sdp_mline_index: Some(0),
            },
            &probe,
        )
        .unwrap();
    avatar.speak_text(&session, "Привет", &probe).unwrap();
    avatar.interrupt(&session, &probe).unwrap();
    avatar.close_session(&session).unwrap();

    let requests: Vec<String> = (0..6).map(|_| captured.recv().unwrap()).collect();
    assert!(requests[0].starts_with("POST /v1/avatar/sessions "));
    assert!(requests[1].starts_with("POST /v1/avatar/sessions/local-1/answer "));
    assert!(requests[2].starts_with("POST /v1/avatar/sessions/local-1/ice "));
    assert!(requests[3].starts_with("POST /v1/avatar/sessions/local-1/speak "));
    assert!(requests[4].starts_with("POST /v1/avatar/sessions/local-1/interrupt "));
    assert!(requests[5].starts_with("DELETE /v1/avatar/sessions/local-1 "));
    assert!(requests.iter().all(|request| {
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer worker-secret")
    }));
    assert!(requests[3].contains("Привет"));
}

#[test]
fn remote_plain_http_is_rejected() {
    let error = LocalOpenSourceAvatar::new(LocalOpenSourceAvatarConfig::new(
        "http://example.com",
        "token",
    ))
    .err()
    .expect("remote plaintext must fail");
    assert_eq!(error.kind, ProviderErrorKind::PolicyDenied);
}

#[test]
fn health_probe_is_authenticated_and_session_free() {
    let (endpoint, captured) = serve(vec![("200 OK", "{}".to_owned())]);
    provider(endpoint).probe_health().unwrap();
    let request = captured.recv().unwrap();
    assert!(request.starts_with("GET /v1/health "));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer worker-secret")
    );
}

#[test]
fn cancellation_prevents_network_work() {
    let avatar = provider("http://127.0.0.1:1".to_owned());
    let error = avatar
        .create_session(&Probe(AtomicBool::new(true)))
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Cancelled);
}

#[test]
fn failed_create_http_response_must_not_allow_duplicate_allocation_retry() {
    // A worker can allocate a remote sender and still reply with HTTP 503.
    // Retrying that CREATE is unsafe without worker-side idempotency.
    for status in ["500 Internal Server Error", "503 Service Unavailable"] {
        let (endpoint, captured) = serve(vec![(status, "{}".to_owned())]);
        let error = provider(endpoint)
            .create_session(&Probe(AtomicBool::new(false)))
            .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::Unavailable);
        assert!(!error.retryable);
        assert!(
            captured
                .recv()
                .unwrap()
                .starts_with("POST /v1/avatar/sessions ")
        );
    }
}

#[test]
fn remote_allocation_without_session_id_times_out_and_cannot_be_admitted() {
    // The worker has accepted CREATE but withheld its allocation receipt.
    // Without a session id the client cannot address a compensating STOP.
    // This RT0 compatibility route must fail closed; RT1 bounded startup
    // remains unavailable until the worker provides a cancellation contract.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        assert!(read_request(&mut stream).starts_with("POST /v1/avatar/sessions "));
        thread::sleep(Duration::from_millis(300));
        let _ = stream.write_all(
            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    });
    let avatar = LocalOpenSourceAvatar::new(
        LocalOpenSourceAvatarConfig::new(format!("http://{address}"), "worker-secret")
            .with_timeout(Duration::from_millis(40)),
    )
    .unwrap();
    let started = std::time::Instant::now();
    let result = avatar.create_session(&Probe(AtomicBool::new(false)));
    let error = result.unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Timeout);
    // A timeout does not prove the remote worker failed to allocate.
    // Repeating CREATE could create a second uncontrolled media sender.
    assert!(!error.retryable);
    assert!(started.elapsed() < Duration::from_millis(250));
    server.join().unwrap();
}

fn test_session() -> RealtimeAvatarSession {
    RealtimeAvatarSession {
        provider_resource_id: "local-1".into(),
        provider_session_id: "local-1".into(),
        transport: RealtimeAvatarTransport::WebRtc {
            offer: WebRtcSessionDescription {
                kind: "offer".into(),
                sdp: "test-offer".into(),
            },
            ice_servers: Vec::new(),
        },
    }
}

#[test]
fn ordinary_close_requires_positive_remote_stop_receipt() {
    for receipt in [
        r"{}",
        r#"{"session_id":"local-1","audio_stopped":true,"video_stopped":false}"#,
        r#"{"session_id":"different","audio_stopped":true,"video_stopped":true}"#,
    ] {
        let (endpoint, captured) = serve(vec![("200 OK", receipt.to_owned())]);
        let error = provider(endpoint)
            .close_session(&test_session())
            .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
        assert!(!error.retryable);
        assert!(
            captured
                .recv()
                .unwrap()
                .starts_with("DELETE /v1/avatar/sessions/local-1 ")
        );
    }
}

#[test]
fn remote_stop_requires_audio_video_and_session_receipt() {
    let (endpoint, captured) = serve(vec![(
        "200 OK",
        r#"{"session_id":"local-1","audio_stopped":true,"video_stopped":true}"#.into(),
    )]);
    provider(endpoint)
        .stop_session_confirmed(
            &test_session(),
            std::time::Instant::now() + std::time::Duration::from_secs(2),
        )
        .unwrap();
    assert!(
        captured
            .recv()
            .unwrap()
            .starts_with("DELETE /v1/avatar/sessions/local-1 ")
    );
}

#[test]
fn remote_stop_rejects_plain_success_and_partial_stop() {
    for receipt in [
        "{}",
        r#"{"session_id":"local-1","audio_stopped":true,"video_stopped":false}"#,
        r#"{"session_id":"other","audio_stopped":true,"video_stopped":true}"#,
    ] {
        let (endpoint, captured) = serve(vec![("200 OK", receipt.into())]);
        assert!(
            provider(endpoint)
                .stop_session_confirmed(
                    &test_session(),
                    std::time::Instant::now() + std::time::Duration::from_secs(2),
                )
                .is_err()
        );
        assert!(captured.recv().unwrap().starts_with("DELETE "));
    }
}

#[test]
fn remote_stop_rejects_false_acknowledgements_for_both_media() {
    // An acknowledgement must confirm BOTH senders. Null/false/string
    // values or a receipt for a different session cannot authorize release.
    for response in [
        r#"{"session_id":"local-1","audio_stopped":false,"video_stopped":true}"#,
        r#"{"session_id":"local-1","audio_stopped":true,"video_stopped":false}"#,
        r#"{"session_id":"local-1","audio_stopped":null,"video_stopped":true}"#,
        r#"{"session_id":"local-1","audio_stopped":"true","video_stopped":true}"#,
        r#"{"session_id":"other","audio_stopped":true,"video_stopped":true}"#,
    ] {
        let (endpoint, captured) = serve(vec![("200 OK", response.to_owned())]);
        let error = provider(endpoint)
            .stop_session_confirmed(
                &test_session(),
                std::time::Instant::now() + Duration::from_secs(2),
            )
            .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
        assert!(captured.recv().unwrap().starts_with("DELETE "));
    }
}

#[test]
fn expired_remote_stop_deadline_never_calls_provider() {
    let avatar = provider("http://127.0.0.1:1".into());
    let err = avatar
        .stop_session_confirmed(&test_session(), std::time::Instant::now())
        .unwrap_err();
    assert_eq!(err.kind, ProviderErrorKind::Timeout);
}

#[test]
fn post_allocation_revoke_attempts_verified_remote_stop_and_never_admits_session() {
    let (endpoint, captured) = serve(vec![(
        "200 OK",
        r#"{"session_id":"local-1","audio_stopped":true,"video_stopped":true}"#.into(),
    )]);
    let avatar = provider(endpoint);
    let cancelled_probe = Probe(AtomicBool::new(true));
    let result = avatar.finalize_created_session(test_session(), &cancelled_probe);
    assert_eq!(result.unwrap_err().kind, ProviderErrorKind::Cancelled);
    assert!(
        captured
            .recv()
            .unwrap()
            .starts_with("DELETE /v1/avatar/sessions/local-1 ")
    );
}

#[test]
fn post_allocation_revoke_never_admits_session_when_remote_stop_is_unconfirmed() {
    let (endpoint, captured) = serve(vec![(
        "200 OK",
        r#"{"session_id":"local-1","audio_stopped":true,"video_stopped":false}"#.into(),
    )]);
    let avatar = provider(endpoint);
    let cancelled_probe = Probe(AtomicBool::new(true));
    let result = avatar.finalize_created_session(test_session(), &cancelled_probe);
    let error = result.unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Unavailable);
    assert!(!error.retryable);
    assert!(
        captured
            .recv()
            .unwrap()
            .starts_with("DELETE /v1/avatar/sessions/local-1 ")
    );
}

#[test]
fn post_allocation_live_authority_does_not_send_cleanup() {
    let avatar = provider("http://127.0.0.1:1".into());
    let active_probe = Probe(AtomicBool::new(false));
    assert_eq!(
        avatar
            .finalize_created_session(test_session(), &active_probe)
            .unwrap()
            .provider_session_id,
        "local-1"
    );
}
