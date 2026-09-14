mod http_evidence;
mod http_owner_capture;
#[cfg(test)]
mod http_security_tests;

use std::env;
use std::error::Error;
use std::io::{Cursor, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use parking_lot::Mutex as ParkingMutex;
use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use vpr_domain::Rt0ReasonCode;
use vpr_integration::{WebRtcIceCandidate, WebRtcSessionDescription};
use vpr_owner_lab::{
    LabError, LabMediaEvidenceInput, LabSessionEvidenceRecorder, OwnerLabEngine,
    OwnerLabStartRequest, OwnerLabTurnInput, ProviderBundle,
};
use vpr_runtime::TurnInterruptHandle;

const MAX_BODY_BYTES: u64 = 128 * 1024;
const MAX_VOICE_BODY_BYTES: u64 = 960_000;
const HTTP_WORKERS: usize = 4;
const DEFAULT_PORT: u16 = 8787;
const HEX: &[u8; 16] = b"0123456789abcdef";
const INDEX_HTML: &str = include_str!("../ui/index.html");
const APP_JS: &str = include_str!("../ui/dist/app.js");
const OWNER_CAPTURE_JS: &str = include_str!("../ui/dist/owner-capture.js");
const STYLES_CSS: &str = include_str!("../ui/styles.css");
const MIC_WORKLET_JS: &str = include_str!("../ui/mic-worklet.js");

type HttpResponse = Response<Cursor<Vec<u8>>>;

struct AppState {
    engine: Mutex<OwnerLabEngine>,
    owner_capture: http_owner_capture::OwnerCaptureHttpState,
    active_voice_interrupt: ParkingMutex<Option<TurnInterruptHandle>>,
    voice_busy: AtomicBool,
    voice_cancel_requested: AtomicBool,
    session_end_requested: AtomicBool,
    evidence: ParkingMutex<LabSessionEvidenceRecorder>,
    csrf_token: String,
    port: u16,
}

#[derive(Serialize)]
struct BootstrapResponse<'a> {
    csrf_token: &'a str,
    egress_enabled: bool,
}

#[derive(Serialize)]
struct ErrorResponse<'a> {
    ok: bool,
    code: &'a str,
}

#[derive(Deserialize)]
struct StartBody {
    consent: bool,
    audience: Option<String>,
}

#[derive(Deserialize)]
struct AnswerBody {
    kind: String,
    sdp: String,
}

#[derive(Deserialize)]
struct IceBody {
    candidate: Option<String>,
    sdp_mid: Option<String>,
    sdp_mline_index: Option<u16>,
}

#[derive(Deserialize)]
struct SpeakBody {
    text: String,
}

#[derive(Deserialize)]
struct AudioBody {
    audio_url: String,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("owner-lab failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let egress_enabled = env::var("VPR_OWNER_LAB_ALLOW_EGRESS").is_ok_and(|value| value == "true");
    let port = env::var("VPR_OWNER_LAB_PORT")
        .ok()
        .map(|value| value.parse::<u16>())
        .transpose()?
        .unwrap_or(DEFAULT_PORT);

    let mut providers = ProviderBundle::from_env(false)?;
    let mut engine = OwnerLabEngine::new(providers.avatar, egress_enabled)
        .map_err(|_| "owner-lab runtime initialization failed")?;
    if let (Some(stt), Some(llm)) = (providers.stt.take(), providers.llm.take()) {
        engine = engine.with_voice(stt, llm);
    }
    let state = Arc::new(AppState {
        engine: Mutex::new(engine),
        owner_capture: http_owner_capture::OwnerCaptureHttpState::default(),
        active_voice_interrupt: ParkingMutex::new(None),
        voice_busy: AtomicBool::new(false),
        voice_cancel_requested: AtomicBool::new(false),
        session_end_requested: AtomicBool::new(false),
        evidence: ParkingMutex::new(LabSessionEvidenceRecorder::default()),
        csrf_token: generate_csrf_token()?,
        port,
    });

    let address = format!("127.0.0.1:{port}");
    let server = Arc::new(Server::http(&address)?);
    println!("VPR Owner Lab: http://{address}");
    if !egress_enabled {
        println!("External provider egress is disabled until VPR_OWNER_LAB_ALLOW_EGRESS=true");
    }
    let workers: Vec<_> = (0..HTTP_WORKERS)
        .map(|_| {
            let server = Arc::clone(&server);
            let state = Arc::clone(&state);
            thread::spawn(move || {
                for request in server.incoming_requests() {
                    handle_request(request, &state);
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().map_err(|_| "owner-lab HTTP worker failed")?;
    }
    Ok(())
}

fn handle_request(mut request: Request, state: &AppState) {
    if !valid_host(&request, state.port) {
        let _ = request.respond(error_response(403, "HOST_DENIED"));
        return;
    }
    let method = request.method().clone();
    let path = request.url().to_owned();
    let response = match (&method, path.as_str()) {
        (&Method::Get, "/") => static_response(INDEX_HTML, "text/html; charset=utf-8"),
        (&Method::Get, "/app.js") => static_response(APP_JS, "text/javascript; charset=utf-8"),
        (&Method::Get, "/owner-capture.js") => {
            static_response(OWNER_CAPTURE_JS, "text/javascript; charset=utf-8")
        }
        (&Method::Get, "/styles.css") => static_response(STYLES_CSS, "text/css; charset=utf-8"),
        (&Method::Get, "/mic-worklet.js") => {
            static_response(MIC_WORKLET_JS, "text/javascript; charset=utf-8")
        }
        (&Method::Get, "/api/bootstrap") => bootstrap_response(state),
        (&Method::Get, "/api/status") => {
            with_engine(state, |engine| json_response(200, &engine.status()))
        }
        (&Method::Get, "/api/persona/capture") => http_owner_capture::snapshot(state),
        (&Method::Get, "/api/evidence/session") => match http_evidence::snapshot(&state.evidence) {
            Ok(snapshot) => json_response(200, &snapshot),
            Err(error) => error_response(http_evidence::error_status(error), error.code()),
        },
        (&Method::Post, "/api/voice/turn") => {
            if valid_voice_post_headers(&request, &state.csrf_token, state.port) {
                voice_turn_response(&mut request, state)
            } else {
                error_response(403, "CSRF_DENIED")
            }
        }
        (&Method::Post, path) if path.starts_with("/api/") => {
            if valid_post_headers(&request, &state.csrf_token, state.port) {
                route_post(path, &mut request, state)
            } else {
                error_response(403, "CSRF_DENIED")
            }
        }
        _ => error_response(404, "NOT_FOUND"),
    };
    let _ = request.respond(response);
}

fn route_post(path: &str, request: &mut Request, state: &AppState) -> HttpResponse {
    if let Some(result) = http_owner_capture::route_post(path, request, state) {
        return result.unwrap_or_else(|response| response);
    }
    match path {
        "/api/avatar/start" => reject_if_session_ending(state).and_then(|()| {
            parse_json::<StartBody>(request).and_then(|body| {
                with_engine_result(state, |engine| {
                    let start_request = OwnerLabStartRequest {
                        consent: body.consent,
                    };
                    let bundle = match body.audience.as_deref().unwrap_or("owner") {
                        "owner" => engine.start(start_request),
                        "visitor" => engine.start_visitor(start_request),
                        _ => Err(LabError::InvalidInput),
                    }?;
                    state
                        .evidence
                        .lock()
                        .begin_session(bundle.evidence_session_sequence)
                        .map_err(|_| LabError::Internal)?;
                    Ok(json_response(200, &bundle))
                })
            })
        }),
        "/api/avatar/answer" => parse_json::<AnswerBody>(request).and_then(|body| {
            apply_input(
                state,
                OwnerLabTurnInput::Answer(WebRtcSessionDescription {
                    kind: body.kind,
                    sdp: body.sdp,
                }),
            )
        }),
        "/api/avatar/ice" => parse_json::<IceBody>(request).and_then(|body| {
            apply_input(
                state,
                OwnerLabTurnInput::Ice(WebRtcIceCandidate {
                    candidate: body.candidate,
                    sdp_mid: body.sdp_mid,
                    sdp_mline_index: body.sdp_mline_index,
                }),
            )
        }),
        "/api/avatar/speak" => parse_json::<SpeakBody>(request)
            .and_then(|body| apply_input(state, OwnerLabTurnInput::Text(body.text))),
        "/api/avatar/audio" => parse_json::<AudioBody>(request)
            .and_then(|body| apply_input(state, OwnerLabTurnInput::AudioUrl(body.audio_url))),
        "/api/evidence/media" => parse_json::<LabMediaEvidenceInput>(request).and_then(|body| {
            http_evidence::record_media(&state.evidence, &body)
                .map(|()| json_response(200, &serde_json::json!({"ok": true})))
                .map_err(|error| error_response(http_evidence::error_status(error), error.code()))
        }),
        "/api/avatar/interrupt" => interrupt_active_turn(state),
        "/api/session/revoke" => end_session(state, false),
        "/api/session/close" => end_session(state, true),
        _ => Ok(error_response(404, "NOT_FOUND")),
    }
    .unwrap_or_else(|response| response)
}

fn reject_if_session_ending(state: &AppState) -> Result<(), HttpResponse> {
    if state.session_end_requested.load(Ordering::Acquire) {
        Err(error_response(409, "INVALID_STATE_TRANSITION"))
    } else {
        Ok(())
    }
}

fn request_voice_cancel(state: &AppState) {
    if !state.voice_busy.load(Ordering::Acquire) {
        return;
    }
    state.voice_cancel_requested.store(true, Ordering::Release);
    if let Some(handle) = state.active_voice_interrupt.lock().clone() {
        let _ = handle.interrupt();
    }
}

fn end_session(state: &AppState, close: bool) -> Result<HttpResponse, HttpResponse> {
    state.session_end_requested.store(true, Ordering::Release);
    state.evidence.lock().seal_session();
    request_voice_cancel(state);
    let mut engine = state
        .engine
        .lock()
        .map_err(|_| error_response(500, "INTERNAL_ERROR"))?;
    let result = if close {
        engine.close()
    } else {
        engine.revoke()
    };
    match result {
        Ok(()) => {
            if close {
                state.session_end_requested.store(false, Ordering::Release);
            }
            Ok(json_response(200, &serde_json::json!({"ok": true})))
        }
        Err(error) => {
            if matches!(engine.status().session_state.as_str(), "none" | "closed") {
                state.session_end_requested.store(false, Ordering::Release);
            }
            Err(lab_error_response(&error))
        }
    }
}

fn voice_turn_response(request: &mut Request, state: &AppState) -> HttpResponse {
    if let Err(response) = reject_if_session_ending(state) {
        return response;
    }
    if state
        .voice_busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return error_response(409, "INVALID_STATE_TRANSITION");
    }
    let _busy =
        http_evidence::VoiceBusyGuard::new(&state.voice_busy, &state.voice_cancel_requested);
    if let Err(response) = reject_if_session_ending(state) {
        return response;
    }
    let request_sequence = match http_evidence::request_sequence(request) {
        Ok(sequence) => sequence,
        Err(error) => return error_response(http_evidence::error_status(error), error.code()),
    };
    let audio = match read_body(request, MAX_VOICE_BODY_BYTES) {
        Ok(audio) => audio,
        Err(response) => return response,
    };
    if let Err(error) = state.evidence.lock().begin_voice_request(request_sequence) {
        return error_response(http_evidence::error_status(error), error.code());
    }
    let Ok(mut engine) = state.engine.lock() else {
        let _ = state
            .evidence
            .lock()
            .fail_voice_request(request_sequence, "INTERNAL_ERROR");
        return error_response(500, "INTERNAL_ERROR");
    };
    let result = engine.voice_turn(audio, |handle| {
        *state.active_voice_interrupt.lock() = Some(handle.clone());
        if state.voice_cancel_requested.load(Ordering::Acquire) {
            let _ = handle.interrupt();
        }
    });
    *state.active_voice_interrupt.lock() = None;
    match result {
        Ok(value) => match state
            .evidence
            .lock()
            .complete_voice_request(request_sequence, &value)
        {
            Ok(()) => json_response(200, &value),
            Err(error) => error_response(http_evidence::error_status(error), error.code()),
        },
        Err(error) => {
            if let Err(evidence_error) = state
                .evidence
                .lock()
                .fail_voice_request(request_sequence, error.code())
            {
                error_response(
                    http_evidence::error_status(evidence_error),
                    evidence_error.code(),
                )
            } else {
                lab_error_response(&error)
            }
        }
    }
}

fn interrupt_active_turn(state: &AppState) -> Result<HttpResponse, HttpResponse> {
    if state.voice_busy.load(Ordering::Acquire) {
        state.voice_cancel_requested.store(true, Ordering::Release);
        if let Some(handle) = state.active_voice_interrupt.lock().clone() {
            return handle
                .interrupt()
                .map(|()| json_response(200, &serde_json::json!({"ok": true})))
                .map_err(|reason| lab_error_response(&LabError::Runtime(reason)));
        }
        return Ok(json_response(200, &serde_json::json!({"ok": true})));
    }
    apply_input(state, OwnerLabTurnInput::Interrupt)
}

fn apply_input(state: &AppState, input: OwnerLabTurnInput) -> Result<HttpResponse, HttpResponse> {
    reject_if_session_ending(state)?;
    with_engine_result(state, |engine| {
        engine
            .apply(input)
            .map(|()| json_response(200, &serde_json::json!({"ok": true})))
    })
}

fn bootstrap_response(state: &AppState) -> HttpResponse {
    with_engine(state, |engine| {
        json_response(
            200,
            &BootstrapResponse {
                csrf_token: &state.csrf_token,
                egress_enabled: engine.status().egress_enabled,
            },
        )
    })
}

fn with_engine(
    state: &AppState,
    operation: impl FnOnce(&OwnerLabEngine) -> HttpResponse,
) -> HttpResponse {
    match state.engine.lock() {
        Ok(engine) => operation(&engine),
        Err(_) => error_response(500, "INTERNAL_ERROR"),
    }
}

fn with_engine_result(
    state: &AppState,
    operation: impl FnOnce(&mut OwnerLabEngine) -> Result<HttpResponse, LabError>,
) -> Result<HttpResponse, HttpResponse> {
    let mut engine = state
        .engine
        .lock()
        .map_err(|_| error_response(500, "INTERNAL_ERROR"))?;
    operation(&mut engine).map_err(|error| lab_error_response(&error))
}

fn parse_json<T: for<'de> Deserialize<'de>>(request: &mut Request) -> Result<T, HttpResponse> {
    if !is_json(request) {
        return Err(error_response(415, "JSON_REQUIRED"));
    }
    let body = read_body(request, MAX_BODY_BYTES)?;
    serde_json::from_slice(&body).map_err(|_| error_response(400, "INVALID_INPUT"))
}

fn read_body(request: &mut Request, limit: u64) -> Result<Vec<u8>, HttpResponse> {
    let mut body = Vec::new();
    request
        .as_reader()
        .take(limit + 1)
        .read_to_end(&mut body)
        .map_err(|_| error_response(400, "INVALID_INPUT"))?;
    if body.len() as u64 > limit {
        return Err(error_response(413, "BODY_TOO_LARGE"));
    }
    Ok(body)
}

fn valid_post_headers(request: &Request, csrf_token: &str, port: u16) -> bool {
    is_json(request)
        && valid_origin(request, port)
        && header_value(request, "X-VPR-CSRF").is_some_and(|value| value == csrf_token)
}

fn valid_voice_post_headers(request: &Request, csrf_token: &str, port: u16) -> bool {
    is_octet_stream(request)
        && valid_origin(request, port)
        && header_value(request, "X-VPR-CSRF").is_some_and(|value| value == csrf_token)
}

fn is_octet_stream(request: &Request) -> bool {
    header_value(request, "Content-Type")
        .is_some_and(|value| value.eq_ignore_ascii_case("application/octet-stream"))
}

fn valid_host(request: &Request, port: u16) -> bool {
    header_value(request, "Host").is_some_and(|host| valid_host_value(host, port))
}

fn valid_origin(request: &Request, port: u16) -> bool {
    let Some(host) = header_value(request, "Host") else {
        return false;
    };
    let Some(origin) = header_value(request, "Origin") else {
        return false;
    };
    valid_origin_value(host, origin, port)
}

fn valid_host_value(host: &str, port: u16) -> bool {
    host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
}

fn valid_origin_value(host: &str, origin: &str, port: u16) -> bool {
    valid_host_value(host, port) && origin == format!("http://{host}")
}

fn is_json(request: &Request) -> bool {
    header_value(request, "Content-Type")
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("application/json"))
}

fn header_value<'a>(request: &'a Request, name: &'static str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv(name))
        .map(|header| header.value.as_str())
}

fn lab_error_response(error: &LabError) -> HttpResponse {
    let status = match error {
        LabError::EgressDisabled
        | LabError::ConsentRequired
        | LabError::Runtime(
            Rt0ReasonCode::AuthRevoked
            | Rt0ReasonCode::AuthExpired
            | Rt0ReasonCode::AuthScopeDenied,
        ) => 403,
        LabError::InvalidInput => 400,
        LabError::InvalidState | LabError::Runtime(_) => 409,
        LabError::Provider(Rt0ReasonCode::ProviderRateLimited) => 429,
        LabError::Provider(Rt0ReasonCode::ProviderTimeout) => 504,
        LabError::Provider(_) => 502,
        LabError::Internal => 500,
    };
    error_response(status, error.code())
}

fn json_response(status: u16, value: &impl Serialize) -> HttpResponse {
    match serde_json::to_vec(value) {
        Ok(body) => response(status, body, "application/json; charset=utf-8"),
        Err(_) => error_response(500, "INTERNAL_ERROR"),
    }
}

fn error_response(status: u16, code: &str) -> HttpResponse {
    let body = serde_json::to_vec(&ErrorResponse { ok: false, code })
        .unwrap_or_else(|_| b"{\"ok\":false,\"code\":\"INTERNAL_ERROR\"}".to_vec());
    response(status, body, "application/json; charset=utf-8")
}

fn static_response(body: &str, content_type: &str) -> HttpResponse {
    response(200, body.as_bytes().to_vec(), content_type)
}

fn response(status: u16, body: Vec<u8>, content_type: &str) -> HttpResponse {
    let mut response = Response::from_data(body).with_status_code(StatusCode(status));
    for (name, value) in [
        ("Content-Type", content_type),
        ("Cache-Control", "no-store"),
        ("X-Content-Type-Options", "nosniff"),
        ("Referrer-Policy", "no-referrer"),
        ("Cross-Origin-Opener-Policy", "same-origin"),
        ("Cross-Origin-Resource-Policy", "same-origin"),
        ("X-Frame-Options", "DENY"),
        (
            "Content-Security-Policy",
            "default-src 'self'; connect-src 'self'; media-src 'self' blob:; style-src 'self'; script-src 'self'; worker-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
        ),
    ] {
        if let Ok(header) = Header::from_bytes(name, value) {
            response.add_header(header);
        }
    }
    response
}

fn generate_csrf_token() -> Result<String, Box<dyn Error + Send + Sync>> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "secure random source unavailable")?;
    let mut token = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        token.push(char::from(HEX[usize::from(byte >> 4)]));
        token.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(token)
}
