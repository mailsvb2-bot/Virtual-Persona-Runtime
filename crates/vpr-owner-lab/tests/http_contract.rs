use std::fmt::Write as FmtWrite;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

static HTTP_CONTRACT_SERIAL: Mutex<()> = Mutex::new(());

fn serialize_owner_lab_http_contract() -> MutexGuard<'static, ()> {
    HTTP_CONTRACT_SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

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
    String::from_utf8_lossy(&request).into_owned()
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

fn http_bytes(
    port: u16,
    method: &str,
    path: &str,
    host: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> HttpResult {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (name, value) in headers {
        let _ = write!(head, "{name}: {value}\r\n");
    }
    if !body.is_empty() {
        let _ = write!(head, "Content-Length: {}\r\n", body.len());
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
    let mut response = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => response.extend_from_slice(&chunk[..read]),
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::ConnectionReset
                        | ErrorKind::ConnectionAborted
                        | ErrorKind::BrokenPipe
                ) && http_response_complete(&response) =>
            {
                break;
            }
            Err(error) => panic!("HTTP {method} {path} response read failed before a complete response ({} bytes): {error}", response.len()),
        }
    }
    let marker = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    let head = String::from_utf8(response[..marker].to_vec()).unwrap();
    let body = String::from_utf8(response[marker + 4..].to_vec()).unwrap();
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
        body,
    }
}

fn http_response_complete(response: &[u8]) -> bool {
    let Some(header_end) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let headers = String::from_utf8_lossy(&response[..header_end]);
    let Some(content_length) = headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    }) else {
        return false;
    };
    response.len() >= header_end + 4 + content_length
}

fn http(
    port: u16,
    method: &str,
    path: &str,
    host: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> HttpResult {
    http_bytes(port, method, path, host, headers, body.as_bytes())
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

fn post_binary(
    port: u16,
    host: &str,
    csrf: &str,
    path: &str,
    request_sequence: Option<u64>,
    body: &[u8],
) -> HttpResult {
    let origin = format!("http://{host}");
    let request_sequence = request_sequence.map(|value| value.to_string());
    let mut headers = vec![
        ("Content-Type", "application/octet-stream"),
        ("Origin", origin.as_str()),
        ("X-VPR-CSRF", csrf),
    ];
    if let Some(value) = request_sequence.as_deref() {
        headers.push(("X-VPR-Evidence-Request", value));
    }
    http_bytes(port, "POST", path, host, &headers, body)
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
    let invalid_close = post(
        port,
        host,
        csrf,
        "/api/session/close",
        r#"{"unexpected":true}"#,
    );
    assert_eq!(invalid_close.status, 400);
    assert!(invalid_close.body.contains("INVALID_INPUT"));
    let still_active = http(port, "GET", "/api/status", host, &[], "");
    let still_active_json: Value = serde_json::from_str(&still_active.body).unwrap();
    assert_eq!(still_active_json["session_state"], "active");
    assert_eq!(still_active_json["avatar_open"], true);

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
fn http_response_completion_requires_full_declared_body() {
    let complete = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ndone";
    assert!(http_response_complete(complete));
    assert!(!http_response_complete(
        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ndo"
    ));
    assert!(!http_response_complete(
        b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\ndone"
    ));
}

#[test]
fn loopback_owner_lab_drives_runtime_and_did_control_plane_fail_closed() {
    let _serial = serialize_owner_lab_http_contract();
    let (did_endpoint, captured) = mock_did();
    let port = free_port();
    let host = format!("127.0.0.1:{port}");
    let _guard = launch_owner_lab(port, &did_endpoint);
    let csrf = bootstrap(port, &host);
    assert_bad_origin_is_blocked(port, &host, &csrf);
    exercise_browser_flow(port, &host, &csrf);
    assert_provider_sequence(&captured);
}

fn mock_once(content_type: &'static str, body: &'static str) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream);
        let _ = tx.send(request);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
    });
    (format!("http://{address}"), rx)
}

fn mock_did_voice() -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let responses = [
            (
                "201 Created",
                r#"{"id":"stream-voice","session_id":"session-voice","offer":{"type":"offer","sdp":"v=0 voice-offer"},"ice_servers":[{"urls":["stun:127.0.0.1"]}]}"#,
            ),
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

fn launch_owner_lab_voice(
    port: u16,
    did_endpoint: &str,
    stt_endpoint: &str,
    llm_endpoint: &str,
) -> ChildGuard {
    let child = Command::new(env!("CARGO_BIN_EXE_vpr-owner-lab"))
        .env("VPR_DID_ENDPOINT", did_endpoint)
        .env("VPR_DID_API_KEY", "did-integration-secret")
        .env("VPR_DID_AGENT_ID", "agent-voice")
        .env("VPR_OWNER_LAB_ALLOW_EGRESS", "true")
        .env("VPR_OWNER_LAB_PORT", port.to_string())
        .env("VPR_OWNER_LAB_STT_PROVIDER", "openai-transcription")
        .env("VPR_OWNER_LAB_STT_ENDPOINT", stt_endpoint)
        .env("VPR_OWNER_LAB_STT_API_KEY", "stt-integration-secret")
        .env("VPR_OWNER_LAB_STT_MODEL", "stt-contract")
        .env("VPR_OWNER_LAB_LLM_PROVIDER", "openai-compatible")
        .env("VPR_OWNER_LAB_LLM_ENDPOINT", llm_endpoint)
        .env("VPR_OWNER_LAB_LLM_API_KEY", "llm-integration-secret")
        .env("VPR_OWNER_LAB_LLM_MODEL", "llm-contract")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for_server(port);
    ChildGuard(child)
}

#[test]
fn loopback_voice_turn_uses_real_stt_llm_and_avatar_adapters() {
    let _serial = serialize_owner_lab_http_contract();
    let (did_endpoint, did_captured) = mock_did_voice();
    let (stt_base, stt_captured) = mock_once(
        "application/json",
        r#"{"text":"Привет","language":"ru-RU"}"#,
    );
    let llm_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Здравствуйте\"}}],\"usage\":null}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":1}}\n\n",
        "data: [DONE]\n\n"
    );
    let (llm_base, llm_captured) = mock_once("text/event-stream", llm_body);
    let stt_endpoint = format!("{stt_base}/v1/audio/transcriptions");
    let llm_endpoint = format!("{llm_base}/v1/chat/completions");
    let port = free_port();
    let host = format!("127.0.0.1:{port}");
    let _guard = launch_owner_lab_voice(port, &did_endpoint, &stt_endpoint, &llm_endpoint);
    let csrf = bootstrap(port, &host);

    let status = http(port, "GET", "/api/status", &host, &[], "");
    let status_json: Value = serde_json::from_str(&status.body).unwrap();
    assert_eq!(status_json["conversation_readiness"], "text_and_voice");

    let start = post(
        port,
        &host,
        &csrf,
        "/api/avatar/start",
        r#"{"consent":true}"#,
    );
    assert_eq!(start.status, 200);
    let start_json: Value = serde_json::from_str(&start.body).unwrap();
    let evidence_session = start_json["evidence_session_sequence"].as_u64().unwrap();
    assert_eq!(evidence_session, 1);
    assert_eq!(
        post(
            port,
            &host,
            &csrf,
            "/api/avatar/answer",
            r#"{"kind":"answer","sdp":"v=0 browser-answer"}"#,
        )
        .status,
        200
    );

    let pcm = vec![0_u8; 3_200];
    let missing_correlation = post_binary(port, &host, &csrf, "/api/voice/turn", None, &pcm);
    assert_eq!(missing_correlation.status, 400);
    assert!(missing_correlation.body.contains("INVALID_INPUT"));

    let voice = post_binary(port, &host, &csrf, "/api/voice/turn", Some(1), &pcm);
    assert_eq!(voice.status, 200, "{}", voice.body);
    let voice_json: Value = serde_json::from_str(&voice.body).unwrap();
    assert_eq!(voice_json["transcript"], "Привет");
    assert_eq!(voice_json["reply"], "Здравствуйте");
    assert_eq!(voice_json["locale"], "ru-RU");
    assert!(voice_json["evidence_turn_sequence"].as_u64().unwrap() > 0);
    assert!(voice_json["evidence_output_sequence"].as_u64().unwrap() > 0);
    assert_eq!(voice_json["stt_usage"]["input_units"], 100);
    assert_eq!(voice_json["llm_usage"]["input_units"], 7);
    assert_eq!(voice_json["llm_usage"]["output_units"], 1);
    for secret in [
        "did-integration-secret",
        "stt-integration-secret",
        "llm-integration-secret",
        "stream-voice",
        "session-voice",
    ] {
        assert!(!voice.body.contains(secret));
    }

    assert_session_evidence_contract(port, &host, &csrf, evidence_session);

    assert_eq!(
        post(port, &host, &csrf, "/api/session/close", "{}").status,
        200
    );
    let late_media = format!(
        r#"{{"session_sequence":{evidence_session},"request_sequence":null,"kind":"reconnect_restored","elapsed_millis":1}}"#
    );
    assert_eq!(
        post(port, &host, &csrf, "/api/evidence/media", &late_media).status,
        409
    );

    let stt_request = stt_captured.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(stt_request.starts_with("POST /v1/audio/transcriptions "));
    assert!(stt_request.contains("authorization: Bearer stt-integration-secret"));
    assert!(stt_request.contains("filename=\"audio.wav\""));
    assert!(stt_request.contains("RIFF"));

    let llm_request = llm_captured.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(llm_request.starts_with("POST /v1/chat/completions "));
    assert!(llm_request.contains("authorization: Bearer llm-integration-secret"));
    assert!(llm_request.contains("Привет"));
    assert!(llm_request.contains("verified owner data is not available"));

    let did_requests: Vec<String> = (0..4)
        .map(|_| did_captured.recv_timeout(Duration::from_secs(2)).unwrap())
        .collect();
    assert!(did_requests[0].starts_with("POST /agents/agent-voice/streams "));
    assert!(did_requests[1].starts_with("POST /agents/agent-voice/streams/stream-voice/sdp "));
    assert!(did_requests[2].starts_with("POST /agents/agent-voice/streams/stream-voice "));
    assert!(did_requests[2].contains("Здравствуйте"));
    assert!(did_requests[3].starts_with("DELETE /agents/agent-voice/streams/stream-voice "));
}

fn assert_session_evidence_contract(port: u16, host: &str, csrf: &str, evidence_session: u64) {
    let media_body = format!(
        r#"{{"session_sequence":{evidence_session},"request_sequence":1,"kind":"audio_started","elapsed_millis":420}}"#
    );
    assert_eq!(
        post(port, host, csrf, "/api/evidence/media", &media_body).status,
        200
    );
    assert_eq!(
        post(port, host, csrf, "/api/evidence/media", &media_body).status,
        409
    );
    let av_sync_body = format!(
        r#"{{"session_sequence":{evidence_session},"request_sequence":1,"sample_sequence":1,"reference":"web_rtc_estimated_playout_timestamp","absolute_offset_millis":60}}"#
    );
    assert_eq!(
        post(port, host, csrf, "/api/evidence/av-sync", &av_sync_body).status,
        200
    );
    assert_eq!(
        post(port, host, csrf, "/api/evidence/av-sync", &av_sync_body).status,
        409
    );
    for sample_sequence in [2, 3] {
        let body = format!(
            r#"{{"session_sequence":{evidence_session},"request_sequence":1,"sample_sequence":{sample_sequence},"reference":"web_rtc_estimated_playout_timestamp","absolute_offset_millis":60}}"#
        );
        assert_eq!(
            post(port, host, csrf, "/api/evidence/av-sync", &body).status,
            200
        );
    }
    let stale_media = format!(
        r#"{{"session_sequence":{},"request_sequence":null,"kind":"video_ready","elapsed_millis":10}}"#,
        evidence_session + 1
    );
    assert_eq!(
        post(port, host, csrf, "/api/evidence/media", &stale_media).status,
        409
    );

    let evidence = http(port, "GET", "/api/evidence/session", host, &[], "");
    assert_eq!(evidence.status, 200, "{}", evidence.body);
    let evidence_json: Value = serde_json::from_str(&evidence.body).unwrap();
    assert_eq!(evidence_json["session_sequence"], evidence_session);
    assert_eq!(evidence_json["scope"], "browser_observed_media_plane_only");
    assert_eq!(evidence_json["canonical_playback_proven"], true);
    assert_eq!(evidence_json["av_sync_proven"], true);
    assert_eq!(
        evidence_json["av_sync_samples"].as_array().unwrap().len(),
        3
    );
    assert_eq!(evidence_json["av_sync_samples"][0]["request_sequence"], 1);
    assert_eq!(
        evidence_json["av_sync_samples"][0]["reference"],
        "web_rtc_estimated_playout_timestamp"
    );
    assert_eq!(
        evidence_json["av_sync_samples"][0]["absolute_offset_millis"],
        60
    );
    assert_eq!(evidence_json["voice_attempts"][0]["request_sequence"], 1);
    assert!(
        evidence_json["voice_attempts"][0]["canonical_turn_sequence"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(
        evidence_json["voice_attempts"][0]["canonical_output_sequence"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(
        evidence_json["voice_attempts"][0]["canonical_playback_confirmed"],
        true
    );
    assert_eq!(evidence_json["voice_attempts"][0]["status"], "completed");
    assert_eq!(evidence_json["media_events"][0]["kind"], "audio_started");
    for private in [
        "Привет",
        "Здравствуйте",
        "did-integration-secret",
        "stream-voice",
    ] {
        assert!(!evidence.body.contains(private));
    }
}

fn mock_did_revoke_during_voice() -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let responses = [
            (
                "201 Created",
                r#"{"id":"stream-revoke","session_id":"session-revoke","offer":{"type":"offer","sdp":"v=0 revoke-offer"},"ice_servers":[]}"#,
            ),
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

fn mock_heartbeat_llm() -> (String, mpsc::Receiver<String>, mpsc::Receiver<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (request_tx, request_rx) = mpsc::channel();
    let (started_tx, started_rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream);
        let _ = request_tx.send(request);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        stream
            .write_all(b"data: {\"choices\":[{\"delta\":{\"content\":\"never-spoken\"}}],\"usage\":null}\n\n")
            .unwrap();
        stream.flush().unwrap();
        let _ = started_tx.send(());
        let deadline = Instant::now() + Duration::from_secs(4);
        while Instant::now() < deadline {
            if stream.write_all(b": keepalive\n\n").is_err() || stream.flush().is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
    });
    (
        format!("http://{address}/v1/chat/completions"),
        request_rx,
        started_rx,
    )
}

#[test]
fn revoke_preempts_active_voice_before_any_avatar_output() {
    let _serial = serialize_owner_lab_http_contract();
    let (did_endpoint, did_captured) = mock_did_revoke_during_voice();
    let (stt_base, _) = mock_once(
        "application/json",
        r#"{"text":"Отмени ответ","language":"ru-RU"}"#,
    );
    let (llm_endpoint, llm_captured, llm_started) = mock_heartbeat_llm();
    let stt_endpoint = format!("{stt_base}/v1/audio/transcriptions");
    let port = free_port();
    let host = format!("127.0.0.1:{port}");
    let _guard = launch_owner_lab_voice(port, &did_endpoint, &stt_endpoint, &llm_endpoint);
    let csrf = bootstrap(port, &host);
    assert_eq!(
        post(
            port,
            &host,
            &csrf,
            "/api/avatar/start",
            r#"{"consent":true}"#,
        )
        .status,
        200
    );

    let voice_host = host.clone();
    let voice_csrf = csrf.clone();
    let voice = thread::spawn(move || {
        post_binary(
            port,
            &voice_host,
            &voice_csrf,
            "/api/voice/turn",
            Some(1),
            &vec![0_u8; 3_200],
        )
    });
    llm_started.recv_timeout(Duration::from_secs(2)).unwrap();

    let revoke_started = Instant::now();
    let revoke = post(port, &host, &csrf, "/api/session/revoke", "{}");
    assert_eq!(revoke.status, 200, "{}", revoke.body);
    assert!(
        revoke_started.elapsed() < Duration::from_millis(1_500),
        "revoke waited for the provider instead of preempting the voice turn"
    );

    let voice = voice.join().unwrap();
    assert_eq!(voice.status, 409, "{}", voice.body);
    assert!(voice.body.contains("TURN_CANCELLED"));
    assert_eq!(
        post(port, &host, &csrf, "/api/session/close", "{}").status,
        200
    );

    let llm_request = llm_captured.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(llm_request.contains("Отмени ответ"));
    let did_requests: Vec<String> = (0..2)
        .map(|_| did_captured.recv_timeout(Duration::from_secs(2)).unwrap())
        .collect();
    assert!(did_requests[0].starts_with("POST /agents/agent-voice/streams "));
    assert!(did_requests[1].starts_with("DELETE /agents/agent-voice/streams/stream-revoke "));
    assert!(
        did_captured
            .recv_timeout(Duration::from_millis(100))
            .is_err()
    );
}

#[test]
fn close_preempts_active_voice_before_any_avatar_output() {
    let _serial = serialize_owner_lab_http_contract();
    let (did_endpoint, did_captured) = mock_did_revoke_during_voice();
    let (stt_base, _) = mock_once(
        "application/json",
        r#"{"text":"Закрой сессию","language":"ru-RU"}"#,
    );
    let (llm_endpoint, llm_captured, llm_started) = mock_heartbeat_llm();
    let stt_endpoint = format!("{stt_base}/v1/audio/transcriptions");
    let port = free_port();
    let host = format!("127.0.0.1:{port}");
    let _guard = launch_owner_lab_voice(port, &did_endpoint, &stt_endpoint, &llm_endpoint);
    let csrf = bootstrap(port, &host);
    assert_eq!(
        post(
            port,
            &host,
            &csrf,
            "/api/avatar/start",
            r#"{"consent":true}"#,
        )
        .status,
        200
    );

    let voice_host = host.clone();
    let voice_csrf = csrf.clone();
    let voice = thread::spawn(move || {
        post_binary(
            port,
            &voice_host,
            &voice_csrf,
            "/api/voice/turn",
            Some(1),
            &vec![0_u8; 3_200],
        )
    });
    llm_started.recv_timeout(Duration::from_secs(2)).unwrap();

    let close_started = Instant::now();
    let close = post(port, &host, &csrf, "/api/session/close", "{}");
    assert_eq!(close.status, 200, "{}", close.body);
    assert!(
        close_started.elapsed() < Duration::from_millis(1_500),
        "close waited for the provider instead of preempting the voice turn"
    );

    let voice = voice.join().unwrap();
    assert_eq!(voice.status, 409, "{}", voice.body);
    assert!(voice.body.contains("TURN_CANCELLED"));

    let status = http(port, "GET", "/api/status", &host, &[], "");
    let status_json: Value = serde_json::from_str(&status.body).unwrap();
    assert_eq!(status_json["session_state"], "closed");
    assert_eq!(status_json["avatar_open"], false);

    let llm_request = llm_captured.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(llm_request.contains("Закрой сессию"));
    let did_requests: Vec<String> = (0..2)
        .map(|_| did_captured.recv_timeout(Duration::from_secs(2)).unwrap())
        .collect();
    assert!(did_requests[0].starts_with("POST /agents/agent-voice/streams "));
    assert!(did_requests[1].starts_with("DELETE /agents/agent-voice/streams/stream-revoke "));
    assert!(
        did_captured
            .recv_timeout(Duration::from_millis(100))
            .is_err()
    );
}
