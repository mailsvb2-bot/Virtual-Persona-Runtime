use std::fmt::Write as FmtWrite;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::Value;

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct HttpResult {
    status: u16,
    headers: String,
    body: String,
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn read_http_request(stream: &mut TcpStream) -> String {
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
            let length = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            });
            total = length.map(|length| header_end + 4 + length);
        }
        if total.is_some_and(|expected| request.len() >= expected) {
            break;
        }
    }
    String::from_utf8(request).unwrap()
}

fn mock_did() -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let responses = [
            (
                "201 Created",
                r#"{"id":"stream-1","session_id":"session-1","offer":{"type":"offer","sdp":"v=0 mock-offer"},"ice_servers":[{"urls":["stun:127.0.0.1"],"username":"u","credential":"c"}]}"#,
            ),
            ("200 OK", "{}"),
            ("200 OK", "{}"),
            ("200 OK", "{}"),
            ("200 OK", "{}"),
        ];
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            let _ = tx.send(request);
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
    });
    (format!("http://{address}"), rx)
}

fn http(
    port: u16,
    method: &str,
    path: &str,
    host: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> HttpResult {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (name, value) in headers {
        let _ = write!(request, "{name}: {value}\r\n");
    }
    if !body.is_empty() {
        let _ = write!(request, "Content-Length: {}\r\n", body.len());
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let status = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse::<u16>()
        .unwrap();
    HttpResult {
        status,
        headers: head.to_ascii_lowercase(),
        body: body.to_owned(),
    }
}

fn wait_for_server(port: u16) {
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("owner lab did not bind loopback port {port}");
}

fn post(port: u16, host: &str, csrf: &str, path: &str, body: &str) -> HttpResult {
    let origin = format!("http://{host}");
    http(
        port,
        "POST",
        path,
        host,
        &[
            ("Content-Type", "application/json"),
            ("Origin", &origin),
            ("X-VPR-CSRF", csrf),
        ],
        body,
    )
}

fn launch_owner_lab(port: u16, did_endpoint: &str) -> ChildGuard {
    let child = Command::new(env!("CARGO_BIN_EXE_vpr-owner-lab"))
        .env("VPR_DID_ENDPOINT", did_endpoint)
        .env("VPR_DID_API_KEY", "integration-secret")
        .env("VPR_DID_AGENT_ID", "agent-1")
        .env("VPR_OWNER_LAB_ALLOW_EGRESS", "true")
        .env("VPR_OWNER_LAB_PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for_server(port);
    ChildGuard(child)
}

fn bootstrap(port: u16, host: &str) -> String {
    let denied = http(port, "GET", "/api/status", "localhost.evil.test", &[], "");
    assert_eq!(denied.status, 403);
    assert!(denied.body.contains("HOST_DENIED"));

    let bootstrap = http(port, "GET", "/api/bootstrap", host, &[], "");
    assert_eq!(bootstrap.status, 200);
    assert!(bootstrap.headers.contains("content-security-policy:"));
    assert!(bootstrap.headers.contains("cache-control: no-store"));
    assert!(!bootstrap.body.contains("integration-secret"));
    let bootstrap_json: Value = serde_json::from_str(&bootstrap.body).unwrap();
    assert_eq!(bootstrap_json["egress_enabled"], true);
    let csrf = bootstrap_json["csrf_token"].as_str().unwrap();
    assert_eq!(csrf.len(), 64);
    csrf.to_owned()
}

fn assert_bad_origin_is_blocked(port: u16, host: &str, csrf: &str) {
    let bad_origin = http(
        port,
        "POST",
        "/api/avatar/start",
        host,
        &[
            ("Content-Type", "application/json"),
            ("Origin", "http://evil.test"),
            ("X-VPR-CSRF", csrf),
        ],
        r#"{"consent":true}"#,
    );
    assert_eq!(bad_origin.status, 403);
    assert!(bad_origin.body.contains("CSRF_DENIED"));
}

fn exercise_browser_flow(port: u16, host: &str, csrf: &str) {
    let start = post(port, host, csrf, "/api/avatar/start", r#"{"consent":true}"#);
    assert_eq!(start.status, 200);
    assert!(start.body.contains("mock-offer"));
    assert!(!start.body.contains("stream-1"));
    assert!(!start.body.contains("session-1"));

    assert_eq!(
        post(
            port,
            host,
            csrf,
            "/api/avatar/answer",
            r#"{"kind":"answer","sdp":"v=0 browser-answer"}"#,
        )
        .status,
        200
    );
    assert_eq!(
        post(
            port,
            host,
            csrf,
            "/api/avatar/ice",
            r#"{"candidate":"candidate:1","sdp_mid":"0","sdp_mline_index":0}"#,
        )
        .status,
        200
    );
    assert_eq!(
        post(
            port,
            host,
            csrf,
            "/api/avatar/speak",
            r#"{"text":"Привет"}"#
        )
        .status,
        200
    );
    assert_eq!(
        post(port, host, csrf, "/api/session/revoke", "{}").status,
        200
    );

    let after_revoke = post(port, host, csrf, "/api/avatar/speak", r#"{"text":"late"}"#);
    assert_eq!(after_revoke.status, 409);
    assert!(after_revoke.body.contains("INVALID_STATE_TRANSITION"));

    let status = http(port, "GET", "/api/status", host, &[], "");
    assert_eq!(status.status, 200);
    let status_json: Value = serde_json::from_str(&status.body).unwrap();
    assert_eq!(status_json["session_state"], "revoked");
    assert_eq!(status_json["avatar_open"], false);
    assert_eq!(
        post(port, host, csrf, "/api/session/close", "{}").status,
        200
    );
}

fn assert_provider_sequence(captured: &mpsc::Receiver<String>) {
    let provider_requests: Vec<String> = (0..5)
        .map(|_| captured.recv_timeout(Duration::from_secs(2)).unwrap())
        .collect();
    assert!(provider_requests[0].starts_with("POST /agents/agent-1/streams "));
    assert!(provider_requests[1].starts_with("POST /agents/agent-1/streams/stream-1/sdp "));
    assert!(provider_requests[2].starts_with("POST /agents/agent-1/streams/stream-1/ice "));
    assert!(provider_requests[3].starts_with("POST /agents/agent-1/streams/stream-1 "));
    assert!(provider_requests[3].contains("Привет"));
    assert!(provider_requests[4].starts_with("DELETE /agents/agent-1/streams/stream-1 "));
    assert!(provider_requests.iter().all(|request| {
        request
            .to_ascii_lowercase()
            .contains("authorization: basic integration-secret")
    }));
    assert!(captured.recv_timeout(Duration::from_millis(100)).is_err());
}

#[test]
fn loopback_owner_lab_drives_runtime_and_did_control_plane_fail_closed() {
    let (did_endpoint, captured) = mock_did();
    let port = free_port();
    let host = format!("127.0.0.1:{port}");
    let _guard = launch_owner_lab(port, &did_endpoint);
    let csrf = bootstrap(port, &host);
    assert_bad_origin_is_blocked(port, &host, &csrf);
    exercise_browser_flow(port, &host, &csrf);
    assert_provider_sequence(&captured);
}
