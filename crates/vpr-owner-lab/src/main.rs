mod evidence_provenance;
mod http_avatar_input;
mod http_client_control;
mod http_evidence;
mod http_json;
mod http_interrupt;
mod http_owner_capture;
mod http_references;
#[cfg(test)]
mod http_security_tests;
mod http_session;
mod http_text;
mod http_voice;
mod launch;

use std::env;
use std::error::Error;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use http_json::{parse_json, read_body};
use http_session::reject_if_session_ending;
use parking_lot::Mutex as ParkingMutex;
use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use vpr_domain::Rt0ReasonCode;
use vpr_integration::{WebRtcIceCandidate, WebRtcSessionDescription};
use vpr_owner_lab::{
    LabError, LabSessionEvidenceRecorder, LabVoicePlaybackRegistry, OwnerLabEngine,
    OwnerLabStartRequest, OwnerLabTurnInput, ParticipantRole, ProviderBundle,
    restore_reviewed_persona,
};
use vpr_runtime::{RealtimeAvatarStopHandle, SessionRevocationHandle, TurnInterruptHandle};
const MAX_BODY_BYTES: u64 = 128 * 1024;
const MAX_VOICE_BODY_BYTES: u64 = 960_000;
const HTTP_WORKERS: usize = 4;
const DEFAULT_PORT: u16 = 8787;
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; connect-src 'self' https: wss:; media-src 'self' blob:; style-src 'self'; script-src 'self'; worker-src 'self' blob:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'";
const HEX: &[u8; 16] = b"0123456789abcdef";
const INDEX_HTML: &str = include_str!("../ui/index.html");
const APP_JS: &str = include_str!("../ui/dist/app.js");
const INTERRUPTED_ANSWER_JS: &str = include_str!("../ui/dist/interrupted-answer.js");
const OWNER_CAPTURE_JS: &str = include_str!("../ui/dist/owner-capture.js");
const VOICE_COMMAND_SCHEDULER_JS: &str = include_str!("../ui/dist/voice-command-scheduler.js");
const BOOTSTRAP_CONTEXT_JS: &str = include_str!("../ui/dist/bootstrap-context.js");
const MEDIA_RUNTIME_JS: &str = include_str!("../ui/dist/media-runtime.js");
const SESSION_RUNTIME_STATE_JS: &str = include_str!("../ui/dist/session-runtime-state.js");
const EVIDENCE_EXPORT_JS: &str = include_str!("../ui/dist/evidence-export.js");
const LIVEKIT_CLIENT_JS: &str = include_str!("../ui/dist/vendor/livekit-client.umd.js");
const LIVEKIT_CLIENT_SHA256: &str = include_str!("../ui/dist/vendor/livekit-client.umd.js.sha256");
const REFERENCE_CAPTURE_JS: &str = include_str!("../ui/reference-capture.js");
const STYLES_CSS: &str = include_str!("../ui/styles.css");
const MIC_WORKLET_JS: &str = include_str!("../ui/mic-worklet.js");
type HttpResponse = Response<Cursor<Vec<u8>>>;
struct AppState {
    engine: Mutex<OwnerLabEngine>,
    rt0_evidence_mode: bool,
    owner_capture: http_owner_capture::OwnerCaptureHttpState,
    reference_intake: http_references::ReferenceIntakeState,
    active_voice_interrupt: ParkingMutex<Option<TurnInterruptHandle>>,
    voice_busy: AtomicBool,
    voice_cancel_requested: AtomicBool,
    voice_inputs: http_voice::VoiceInputRegistry,
    voice_streams: http_voice::VoiceStreamRegistry,
    voice_playback: LabVoicePlaybackRegistry,
    replay_source: ParkingMutex<Option<http_client_control::AuthorizedReply>>,
    session_end_requested: AtomicBool,
    session_revocation: ParkingMutex<Option<SessionRevocationHandle>>,
    backend_stop: ParkingMutex<Option<RealtimeAvatarStopHandle>>,
    active_session_sequence: AtomicU64,
    evidence: ParkingMutex<LabSessionEvidenceRecorder>,
    evidence_export: http_evidence::EvidenceExportTracker,
    csrf_token: String,
    port: u16,
}
#[derive(Serialize)]
struct BootstrapResponse<'a> {
    csrf_token: &'a str,
    egress_enabled: bool,
    rt0_evidence_mode: bool,
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

fn main() {
    if let Err(error) = run() {
        eprintln!("owner-lab failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let launch_options = launch::parse_launch_options(env::args().skip(1))
        .map_err(|error| format!("invalid owner-lab launch options: {error}"))?;
    if launch_options.show_help {
        launch::print_usage();
        return Ok(());
    }

    let egress_env = env::var("VPR_OWNER_LAB_ALLOW_EGRESS").ok();
    let egress_enabled = launch::resolve_egress_enabled(&launch_options, egress_env.as_deref());
    let rt0_evidence_mode = match env::var("VPR_OWNER_LAB_RT0_EVIDENCE") {
        Ok(value) if value.eq_ignore_ascii_case("true") => true,
        Ok(value) if value.eq_ignore_ascii_case("false") => false,
        Ok(_) => return Err("VPR_OWNER_LAB_RT0_EVIDENCE must be true or false".into()),
        Err(env::VarError::NotPresent) => false,
        Err(env::VarError::NotUnicode(_)) => {
            return Err("VPR_OWNER_LAB_RT0_EVIDENCE must be valid text".into());
        }
    };
    let port = env::var("VPR_OWNER_LAB_PORT")
        .ok()
        .map(|value| value.parse::<u16>())
        .transpose()?
        .unwrap_or(DEFAULT_PORT);
    let mut providers = ProviderBundle::from_env(false)?;
    let evidence_provenance = evidence_provenance::resolve(&providers, rt0_evidence_mode)?;

    let mut engine = OwnerLabEngine::new(providers.avatar, egress_enabled)
        .map_err(|_| "owner-lab runtime initialization failed")?;
    if let Some(llm) = providers.llm.take() {
        engine = engine.with_llm(llm);
    }
    if let Some(stt) = providers.stt.take() {
        engine = engine.with_stt(stt);
    }
    if restore_reviewed_persona(&mut engine)? {
        println!("Restored reviewed Persona from persistent store.");
    }
    let voice_playback = engine.voice_playback_registry();
    let mut evidence_recorder = LabSessionEvidenceRecorder::default();
    if let Some((candidate_sha, provider_state_sha256)) = evidence_provenance {
        evidence_recorder
            .bind_provenance(&candidate_sha, &provider_state_sha256)
            .map_err(|_| "owner-lab evidence provenance rejected")?;
        if rt0_evidence_mode {
            println!(
                "RT0 evidence provenance: candidate={candidate_sha} provider_state_sha256={provider_state_sha256}"
            );
        }
    }

    let state = Arc::new(AppState {
        engine: Mutex::new(engine),
        rt0_evidence_mode,
        owner_capture: http_owner_capture::OwnerCaptureHttpState::default(),
        reference_intake: http_references::ReferenceIntakeState::default(),
        active_voice_interrupt: ParkingMutex::new(None),
        voice_busy: AtomicBool::new(false),
        voice_cancel_requested: AtomicBool::new(false),
        voice_inputs: http_voice::VoiceInputRegistry::default(),
        voice_streams: http_voice::VoiceStreamRegistry::default(),
        voice_playback,
        replay_source: ParkingMutex::new(None),
        session_end_requested: AtomicBool::new(false),
        session_revocation: ParkingMutex::new(None),
        backend_stop: ParkingMutex::new(None),
        active_session_sequence: AtomicU64::new(0),
        evidence: ParkingMutex::new(evidence_recorder),
        evidence_export: http_evidence::EvidenceExportTracker::default(),
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

fn handle_request(mut request: Request, state: &Arc<AppState>) {
    if !valid_host(&request, state.port) {
        let _ = request.respond(error_response(403, "HOST_DENIED"));
        return;
    }

    let method = request.method().clone();
    let path = request.url().to_owned();
    let response = if method == Method::Get {
        static_get_response(&path)
            .unwrap_or_else(|| route_request(&method, &path, &mut request, state))
    } else {
        route_request(&method, &path, &mut request, state)
    };
    let _ = request.respond(response);
}

fn static_get_response(path: &str) -> Option<HttpResponse> {
    let response = match path {
        "/" => static_response(INDEX_HTML, "text/html; charset=utf-8"),
        "/app.js" => static_response(APP_JS, "text/javascript; charset=utf-8"),
        "/interrupted-answer.js" => {
            static_response(INTERRUPTED_ANSWER_JS, "text/javascript; charset=utf-8")
        }
        "/owner-capture.js" => static_response(OWNER_CAPTURE_JS, "text/javascript; charset=utf-8"),
        "/voice-command-scheduler.js" => {
            static_response(VOICE_COMMAND_SCHEDULER_JS, "text/javascript; charset=utf-8")
        }
        "/bootstrap-context.js" => {
            static_response(BOOTSTRAP_CONTEXT_JS, "text/javascript; charset=utf-8")
        }
        "/media-runtime.js" => static_response(MEDIA_RUNTIME_JS, "text/javascript; charset=utf-8"),
        "/session-runtime-state.js" => {
            static_response(SESSION_RUNTIME_STATE_JS, "text/javascript; charset=utf-8")
        }
        "/evidence-export.js" => {
            static_response(EVIDENCE_EXPORT_JS, "text/javascript; charset=utf-8")
        }
        "/vendor/livekit-client.umd.js" => {
            static_response(LIVEKIT_CLIENT_JS, "text/javascript; charset=utf-8")
        }
        "/vendor/livekit-client.umd.js.sha256" => {
            static_response(LIVEKIT_CLIENT_SHA256, "text/plain; charset=utf-8")
        }
        "/reference-capture.js" => {
            static_response(REFERENCE_CAPTURE_JS, "text/javascript; charset=utf-8")
        }
        "/styles.css" => static_response(STYLES_CSS, "text/css; charset=utf-8"),
        "/mic-worklet.js" => static_response(MIC_WORKLET_JS, "text/javascript; charset=utf-8"),
        _ => return None,
    };
    Some(response)
}

fn route_request(
    method: &Method,
    path: &str,
    request: &mut Request,
    state: &Arc<AppState>,
) -> HttpResponse {
    match (method, path) {
        (&Method::Get, "/api/bootstrap") => bootstrap_response(state),
        (&Method::Get, "/api/status") => {
            with_engine(state, |engine| json_response(200, &engine.status()))
        }
        (&Method::Get, "/api/persona/capture") => http_owner_capture::snapshot(state),
        (&Method::Get, "/api/references") => {
            http_references::snapshot_response(&state.reference_intake)
        }
        (&Method::Get, "/api/evidence/session") => match http_evidence::snapshot(&state.evidence) {
            Ok(snapshot) => json_response(200, &snapshot),
            Err(error) => error_response(http_evidence::error_status(error), error.code()),
        },
        (&Method::Post, "/api/voice/turn") => {
            if valid_voice_post_headers(request, &state.csrf_token, state.port) {
                http_voice::voice_turn_response(request, state)
            } else {
                error_response(403, "CSRF_DENIED")
            }
        }
        (&Method::Post, "/api/voice/input/chunk") => {
            if valid_voice_post_headers(request, &state.csrf_token, state.port) {
                http_voice::input_chunk_response(request, state)
            } else {
                error_response(403, "CSRF_DENIED")
            }
        }
        (&Method::Post, "/api/voice/input/start") => {
            if valid_post_headers(request, &state.csrf_token, state.port) {
                http_voice::start_input_response(request, state)
            } else {
                error_response(403, "CSRF_DENIED")
            }
        }
        (&Method::Post, "/api/voice/input/finish") => {
            if valid_post_headers(request, &state.csrf_token, state.port) {
                http_voice::finish_input_response(request, state)
            } else {
                error_response(403, "CSRF_DENIED")
            }
        }
        (&Method::Post, "/api/voice/input/cancel") => {
            if valid_post_headers(request, &state.csrf_token, state.port) {
                http_voice::cancel_input_response(request, state)
            } else {
                error_response(403, "CSRF_DENIED")
            }
        }
        (&Method::Post, "/api/references/intake") => {
            if valid_voice_post_headers(request, &state.csrf_token, state.port) {
                http_references::intake_response(request, &state.reference_intake)
                    .unwrap_or_else(|response| response)
            } else {
                error_response(403, "CSRF_DENIED")
            }
        }
        (&Method::Post, api_path) if api_path.starts_with("/api/") => {
            if valid_post_headers(request, &state.csrf_token, state.port) {
                route_post(api_path, request, state)
            } else {
                error_response(403, "CSRF_DENIED")
            }
        }
        _ => error_response(404, "NOT_FOUND"),
    }
}

fn route_post(path: &str, request: &mut Request, state: &Arc<AppState>) -> HttpResponse {
    if let Some(result) = http_owner_capture::route_post(path, request, state) {
        return result.unwrap_or_else(|response| response);
    }
    if let Some(result) = http_client_control::route_post(path, request, state) {
        return result.unwrap_or_else(|response| response);
    }
    if let Some(result) = http_avatar_input::route_post(path, request, state) {
        return result.unwrap_or_else(|response| response);
    }
    if let Some(result) = http_evidence::route_post(path, request, state) {
        return result.unwrap_or_else(|response| response);
    }
    if path == "/api/references/clear" {
        return http_references::clear_response(request, &state.reference_intake)
            .unwrap_or_else(|response| response);
    }
    match path {
        "/api/avatar/start" => http_evidence::ensure_previous_exported(
            &state.engine,
            &state.evidence,
            &state.evidence_export,
        )
        .map_err(|error| error_response(error.status(), error.code()))
        .and_then(|()| reject_if_session_ending(state))
        .and_then(|()| {
            parse_json::<StartBody>(request).and_then(|body| {
                let mut replay_source = state.replay_source.lock();
                reject_if_session_ending(state)?;
                with_engine_result(state, |engine| {
                    let start_request = OwnerLabStartRequest {
                        consent: body.consent,
                    };
                    let (bundle, participant_role) =
                        match body.audience.as_deref().unwrap_or("owner") {
                            "owner" => (engine.start(start_request)?, ParticipantRole::Owner),
                            "visitor" => (
                                engine.start_visitor(start_request)?,
                                ParticipantRole::Visitor,
                            ),
                            _ => return Err(LabError::InvalidInput),
                        };
                    http_session::register_started_session(state, engine)?;
                    *replay_source = None;
                    state
                        .evidence
                        .lock()
                        .begin_session(bundle.evidence_session_sequence, participant_role)
                        .map_err(|_| LabError::Internal)?;
                    let sequence = bundle.evidence_session_sequence;
                    state
                        .active_session_sequence
                        .store(sequence, Ordering::Release);
                    state.voice_streams.clear();
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
        "/api/text/turn" => http_text::text_turn_response(request, state),
        "/api/voice/events" => http_voice::events_response(request, state),
        "/api/avatar/interrupt" => http_interrupt::interrupt_response(request, state),
        "/api/session/fence" => http_session::fence_session_response(request, state),
        "/api/session/revoke" => http_session::end_session_response(request, state, false),
        "/api/session/close" => http_session::end_session_response(request, state, true),
        _ => Ok(error_response(404, "NOT_FOUND")),
    }
    .unwrap_or_else(|response| response)
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
                rt0_evidence_mode: state.rt0_evidence_mode,
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
        LabError::Provider(Rt0ReasonCode::BudgetExhausted) => 402,
        LabError::Provider(Rt0ReasonCode::ProviderRateLimited) => 429,
        LabError::Provider(Rt0ReasonCode::ProviderTimeout) => 504,
        LabError::Provider(_) => 502,
        LabError::PersistenceFailed | LabError::Internal => 500,
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
        ("Content-Security-Policy", CONTENT_SECURITY_POLICY),
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
