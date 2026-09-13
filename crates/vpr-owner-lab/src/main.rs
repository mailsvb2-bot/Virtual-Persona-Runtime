use std::env;
use std::error::Error;
use std::io::{Cursor, Read};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use vpr_domain::Rt0ReasonCode;
use vpr_integration::{WebRtcIceCandidate, WebRtcSessionDescription};
use vpr_owner_lab::{LabError, OwnerLabEngine, OwnerLabStartRequest, OwnerLabTurnInput};
use vpr_provider_did_agent_streams::{DidAgentStreamsAvatar, DidAgentStreamsConfig};

const MAX_BODY_BYTES: u64 = 128 * 1024;
const DEFAULT_PORT: u16 = 8787;
const HEX: &[u8; 16] = b"0123456789abcdef";
const INDEX_HTML: &str = include_str!("../ui/index.html");
const APP_JS: &str = include_str!("../ui/dist/app.js");
const STYLES_CSS: &str = include_str!("../ui/styles.css");

type HttpResponse = Response<Cursor<Vec<u8>>>;

struct AppState {
    engine: Mutex<OwnerLabEngine>,
    csrf_token: String,
    port: u16,
}

#[derive(Serialize)]
struct BootstrapResponse<'a> {
    csrf_token: &'a str,
    egress_enabled: bool,
}

#[derive(Serialize)]
struct OkResponse {
    ok: bool,
}

#[derive(Serialize)]
struct ErrorResponse<'a> {
    ok: bool,
    code: &'a str,
}

#[derive(Deserialize)]
struct StartBody {
    consent: bool,
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
    let endpoint = env::var("VPR_DID_ENDPOINT").unwrap_or_else(|_| "https://api.d-id.com".into());
    let api_key = required_env("VPR_DID_API_KEY")?;
    let agent_id = required_env("VPR_DID_AGENT_ID")?;
    let egress_enabled = env::var("VPR_OWNER_LAB_ALLOW_EGRESS").is_ok_and(|value| value == "true");
    let port = env::var("VPR_OWNER_LAB_PORT")
        .ok()
        .map(|value| value.parse::<u16>())
        .transpose()?
        .unwrap_or(DEFAULT_PORT);

    let provider =
        DidAgentStreamsAvatar::new(DidAgentStreamsConfig::new(endpoint, api_key, agent_id))
            .map_err(|_| "D-ID provider configuration rejected")?;
    let engine = OwnerLabEngine::new(Box::new(provider), egress_enabled)
        .map_err(|_| "owner-lab runtime initialization failed")?;
    let state = Arc::new(AppState {
        engine: Mutex::new(engine),
        csrf_token: generate_csrf_token()?,
        port,
    });

    let address = format!("127.0.0.1:{port}");
    let server = Server::http(&address)?;
    println!("VPR Owner Lab: http://{address}");
    if !egress_enabled {
        println!("External provider egress is disabled until VPR_OWNER_LAB_ALLOW_EGRESS=true");
    }
    for request in server.incoming_requests() {
        handle_request(request, &state);
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
        (&Method::Get, "/styles.css") => static_response(STYLES_CSS, "text/css; charset=utf-8"),
        (&Method::Get, "/api/bootstrap") => bootstrap_response(state),
        (&Method::Get, "/api/status") => {
            with_engine(state, |engine| json_response(200, &engine.status()))
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
    match path {
        "/api/avatar/start" => parse_json::<StartBody>(request).and_then(|body| {
            with_engine_result(state, |engine| {
                engine
                    .start(OwnerLabStartRequest {
                        consent: body.consent,
                    })
                    .map(|bundle| json_response(200, &bundle))
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
        "/api/avatar/interrupt" => apply_input(state, OwnerLabTurnInput::Interrupt),
        "/api/session/revoke" => with_engine_result(state, |engine| {
            engine
                .revoke()
                .map(|()| json_response(200, &OkResponse { ok: true }))
        }),
        "/api/session/close" => with_engine_result(state, |engine| {
            engine
                .close()
                .map(|()| json_response(200, &OkResponse { ok: true }))
        }),
        _ => Ok(error_response(404, "NOT_FOUND")),
    }
    .unwrap_or_else(|response| response)
}

fn apply_input(state: &AppState, input: OwnerLabTurnInput) -> Result<HttpResponse, HttpResponse> {
    with_engine_result(state, |engine| {
        engine
            .apply(input)
            .map(|()| json_response(200, &OkResponse { ok: true }))
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
    let mut body = Vec::new();
    request
        .as_reader()
        .take(MAX_BODY_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|_| error_response(400, "INVALID_INPUT"))?;
    if body.len() as u64 > MAX_BODY_BYTES {
        return Err(error_response(413, "BODY_TOO_LARGE"));
    }
    serde_json::from_slice(&body).map_err(|_| error_response(400, "INVALID_INPUT"))
}

fn valid_post_headers(request: &Request, csrf_token: &str, port: u16) -> bool {
    is_json(request)
        && valid_origin(request, port)
        && header_value(request, "X-VPR-CSRF").is_some_and(|value| value == csrf_token)
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
        | LabError::Runtime(Rt0ReasonCode::AuthRevoked | Rt0ReasonCode::AuthExpired) => 403,
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
            "default-src 'self'; connect-src 'self'; media-src 'self' blob:; style-src 'self'; script-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
        ),
    ] {
        if let Ok(header) = Header::from_bytes(name, value) {
            response.add_header(header);
        }
    }
    response
}

fn required_env(name: &'static str) -> Result<String, Box<dyn Error + Send + Sync>> {
    env::var(name).map_err(|_| format!("required environment variable {name} is not set").into())
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

#[cfg(test)]
mod http_security_tests {
    use super::{valid_host_value, valid_origin_value};

    #[test]
    fn host_validation_rejects_dns_rebinding_shapes() {
        for accepted in ["127.0.0.1:8787", "localhost:8787"] {
            assert!(
                valid_host_value(accepted, 8787),
                "expected accepted host: {accepted}"
            );
        }
        for denied in [
            "127.0.0.1",
            "localhost",
            "localhost:9999",
            "localhost.evil.test",
            "127.0.0.1.evil.test",
            "evil.test",
            "0.0.0.0:8787",
            "[::1]:8787",
        ] {
            assert!(
                !valid_host_value(denied, 8787),
                "expected denied host: {denied}"
            );
        }
    }

    #[test]
    fn post_origin_must_match_the_loopback_host_exactly() {
        assert!(valid_origin_value(
            "127.0.0.1:8787",
            "http://127.0.0.1:8787",
            8787
        ));
        assert!(valid_origin_value(
            "localhost:8787",
            "http://localhost:8787",
            8787
        ));
        assert!(!valid_origin_value(
            "127.0.0.1:8787",
            "http://localhost:8787",
            8787
        ));
        assert!(!valid_origin_value(
            "localhost:8787",
            "https://localhost:8787",
            8787
        ));
        assert!(!valid_origin_value(
            "localhost.evil.test",
            "http://localhost.evil.test",
            8787
        ));
    }
}
