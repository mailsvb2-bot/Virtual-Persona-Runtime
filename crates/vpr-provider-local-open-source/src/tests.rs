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
        ("200 OK", "{}".to_owned()),
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
