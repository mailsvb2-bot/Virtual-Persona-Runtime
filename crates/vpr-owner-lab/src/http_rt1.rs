//! Localhost-only RT1 browser entry. Uses the existing owner workspace and
//! visitor conversation, never the owner RT0 engine as a visitor's authority.
use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Deserialize;
use vpr_owner_lab::{
    OwnerContextState, ProviderBundle, Rt1OwnerController, Rt1VisitorConversation,
    restore_reviewed_persona,
};

use super::{AppState, HttpResponse, error_response, json_response, parse_json};

/// This local-only diagnostic journey is not a released RT1 API.
pub(super) fn enabled() -> bool {
    std::env::var("VPR_RT1_LOCAL_EXPERIMENT").as_deref() == Ok("true")
}

pub(super) fn static_get_response(path: &str) -> Option<HttpResponse> {
    if !enabled() {
        return None;
    }
    match path {
        "/rt1" => Some(super::static_response(
            include_str!("../ui/rt1.html"),
            "text/html; charset=utf-8",
        )),
        "/rt1.js" => Some(super::static_response(
            include_str!("../ui/rt1.js"),
            super::JS_MIME,
        )),
        "/rt1.css" => Some(super::static_response(
            include_str!("../ui/rt1.css"),
            "text/css; charset=utf-8",
        )),
        _ => None,
    }
}

#[derive(Default)]
pub(super) struct Rt1HttpState {
    conversations: Mutex<HashMap<String, Arc<Rt1VisitorConversation>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VisitorStart {
    visitor_token: String,
    consent: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VisitorSession {
    session_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VisitorAnswer {
    session_id: String,
    sdp: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VisitorVoice {
    session_id: String,
    pcm: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VisitorText {
    session_id: String,
    text: String,
}

// S16LE mono/16 kHz, 30s maximum. JSON arrays require up to four ASCII
// bytes per PCM byte. Only the explicitly local, authenticated RT1 route
// accepts this larger request; ordinary owner endpoints keep 128 KiB.
const MAX_RT1_VOICE_BYTES: usize = 16_000 * 2 * 30;
const MAX_RT1_VOICE_JSON_BODY: u64 = (MAX_RT1_VOICE_BYTES * 4 + 4096) as u64;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VisitorIce {
    session_id: String,
    candidate: Option<String>,
    sdp_mid: Option<String>,
    sdp_mline_index: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnerToken {
    owner_token: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnerPublish {
    owner_token: String,
    expected_persona_version: u64,
}

/// The visitor text/voice response must retain its canonical egress permit
/// through the actual HTTP write. Other RT1 routes have no buffered reply.
pub(super) struct Rt1RoutedResponse {
    pub response: HttpResponse,
    pub delivery: Option<Arc<Rt1VisitorConversation>>,
}

fn guarded_reply(
    result: Result<(HttpResponse, Arc<Rt1VisitorConversation>), HttpResponse>,
) -> Rt1RoutedResponse {
    match result {
        Ok((response, conversation)) => Rt1RoutedResponse {
            response,
            delivery: Some(conversation),
        },
        Err(response) => Rt1RoutedResponse {
            response,
            delivery: None,
        },
    }
}

/// Final RT1 HTTP dispatch is owned by this module, including the actual
/// response write. Keeping this here avoids a second path around the CSRF
/// checks and preserves the main server's architectural module-size limit.
pub(super) fn respond_post(mut request: tiny_http::Request, state: &Arc<AppState>) {
    if !super::valid_post_headers(&request, &state.csrf_token, state.port) {
        let _ = request.respond(error_response(403, "CSRF_DENIED"));
        return;
    }
    if !enabled() {
        let _ = request.respond(error_response(404, "NOT_FOUND"));
        return;
    }
    let path = request.url().to_owned();
    let Some(routed) = route_post(&path, &mut request, state) else {
        let _ = request.respond(error_response(404, "NOT_FOUND"));
        return;
    };
    if let Some(conversation) = routed.delivery {
        match conversation.begin_browser_delivery() {
            Ok(_lease) => {
                let _ = request.respond(routed.response);
            }
            Err(_) => {
                let _ = request.respond(error_response(409, "RT1_VISITOR_DELIVERY_DENIED"));
            }
        }
    } else {
        let _ = request.respond(routed.response);
    }
}

pub(super) fn route_post(
    path: &str,
    request: &mut tiny_http::Request,
    state: &Arc<AppState>,
) -> Option<Rt1RoutedResponse> {
    // Never serialize a visitor LLM or microphone result and then drop its
    // authority before the HTTP writer gets the response.
    if path == "/api/rt1/visitor/text" {
        return Some(guarded_reply(visitor_text(request, state)));
    }
    if path == "/api/rt1/visitor/voice" {
        return Some(guarded_reply(visitor_voice(request, state)));
    }
    let result = match path {
        "/api/rt1/owner/provision" => owner_provision(request),
        "/api/rt1/owner/publish" => owner_publish(request, state),
        "/api/rt1/owner/pause" => owner_revoke(request, false),
        "/api/rt1/owner/unpublish" => owner_revoke(request, true),
        "/api/rt1/visitor/start" => visitor_start(request, state),
        "/api/rt1/visitor/answer" => visitor_answer(request, state),
        "/api/rt1/visitor/ice" => visitor_ice(request, state),
        "/api/rt1/visitor/interrupt" => visitor_interrupt(request, state),
        "/api/rt1/visitor/status" => visitor_status(request, state),
        "/api/rt1/visitor/close" => visitor_close(request, state),
        _ => return None,
    };
    Some(Rt1RoutedResponse {
        response: result.unwrap_or_else(|response| response),
        delivery: None,
    })
}

type Rt1HttpResult = Result<HttpResponse, HttpResponse>;

fn owner_provision(request: &mut tiny_http::Request) -> Rt1HttpResult {
    let _: serde_json::Value = parse_json(request)?;
    let (owner_id, owner_token) = Rt1OwnerController::new()
        .provision_owner()
        .map_err(|_| error_response(409, "RT1_OWNER_PROVISION_DENIED"))?;
    Ok(json_response(
        200,
        &serde_json::json!({
            "owner_id": owner_id,
            "owner_token": owner_token,
        }),
    ))
}

fn owner_publish(request: &mut tiny_http::Request, state: &AppState) -> Rt1HttpResult {
    let body: OwnerPublish = parse_json(request)?;
    let readiness = || -> Result<(), String> {
        let engine = state.engine.lock().map_err(|_| "RT1_ENGINE_UNAVAILABLE")?;
        let status = engine.status();
        if !status.egress_enabled || status.owner_context_state != OwnerContextState::Reviewed {
            return Err("RT1_REVIEWED_PERSONA_AND_EGRESS_REQUIRED".into());
        }
        if std::env::var("VPR_RT1_DIAGNOSTIC_RTP").as_deref() == Ok("true") {
            for name in ["RT1_WORKER_ENDPOINT", "RT1_WORKER_TOKEN"] {
                if std::env::var(name)
                    .ok()
                    .is_none_or(|value| value.trim().is_empty())
                {
                    return Err("RT1_WORKER_NOT_CONFIGURED".into());
                }
            }
        }
        Ok(())
    };
    let visitor_token = Rt1OwnerController::new()
        .publish(&body.owner_token, body.expected_persona_version, readiness)
        .map_err(|_| error_response(409, "RT1_PUBLISH_DENIED"))?;
    Ok(json_response(
        200,
        &serde_json::json!({"visitor_token": visitor_token}),
    ))
}

fn owner_revoke(request: &mut tiny_http::Request, unpublish: bool) -> Rt1HttpResult {
    let body: OwnerToken = parse_json(request)?;
    let controller = Rt1OwnerController::new();
    if unpublish {
        controller.unpublish(&body.owner_token)
    } else {
        controller.pause(&body.owner_token)
    }
    .map_err(|_| error_response(403, "RT1_OWNER_ACTION_DENIED"))?;
    // Existing stop callbacks cannot report remote receipts to this call.
    // A durable publication revoke is not a verified media STOP.
    Ok(json_response(
        200,
        &serde_json::json!({
            "ok": true,
            "publication_revoked": true,
            "remote_stop_confirmed": false,
        }),
    ))
}

#[derive(Debug, PartialEq, Eq)]
enum RegistrationFailure {
    Collision,
    Revoked,
}

/// Publish a registration before its final authority check, but release the
/// registry lock before that check can trigger a slow authenticated STOP.
/// A revoked registration remains quarantined until remote STOP is proven:
/// removing it here would discard the only HTTP retry handle.
fn register_visitor<T>(
    sessions: &Mutex<HashMap<String, Arc<T>>>,
    session_id: &str,
    conversation: &Arc<T>,
    active_now: impl FnOnce(&T) -> bool,
) -> Result<(), RegistrationFailure> {
    {
        let mut active = sessions.lock();
        match active.entry(session_id.to_owned()) {
            std::collections::hash_map::Entry::Occupied(_) => {
                return Err(RegistrationFailure::Collision);
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(Arc::clone(conversation));
            }
        }
    }
    if !active_now(conversation.as_ref()) {
        return Err(RegistrationFailure::Revoked);
    }
    Ok(())
}

/// Never discard a quarantined registration on a failed STOP or on a local
/// transport close that lacks independent remote audio/video sender receipts.
/// Identity comparison prevents a late STOP from removing a newer session.
fn finalize_registered_stop<T>(
    sessions: &Mutex<HashMap<String, Arc<T>>>,
    session_id: &str,
    conversation: &Arc<T>,
    stop: Result<(), String>,
    remote_video_receipt_path: bool,
) -> Result<(), String> {
    stop?;
    if !remote_video_receipt_path {
        return Err("RT1_REMOTE_VIDEO_STOP_UNCONFIRMED".into());
    }
    let mut active = sessions.lock();
    if active
        .get(session_id)
        .is_some_and(|current| Arc::ptr_eq(current, conversation))
    {
        active.remove(session_id);
    }
    Ok(())
}

/// Allocate the HTTP retry identity before any remote CREATE. Entropy failure
/// must never strand an already-started provider without a STOP lookup key.
fn new_visitor_session_id() -> Result<String, HttpResponse> {
    let mut raw = [0_u8; 32];
    getrandom::fill(&mut raw)
        .map_err(|_| error_response(503, "RT1_SESSION_ENTROPY_UNAVAILABLE"))?;
    let mut session_id = String::with_capacity(64);
    for byte in raw {
        session_id.push(char::from(super::HEX[usize::from(byte >> 4)]));
        session_id.push(char::from(super::HEX[usize::from(byte & 0x0f)]));
    }
    Ok(session_id)
}

fn visitor_start(request: &mut tiny_http::Request, state: &AppState) -> Rt1HttpResult {
    let body: VisitorStart = parse_json(request)?;
    if !body.consent {
        return Err(error_response(403, "CONSENT_REQUIRED"));
    }
    // Do not call the provider until the retry handle exists.
    let session_id = new_visitor_session_id()?;
    let providers = ProviderBundle::from_env(false)
        .map_err(|_| error_response(503, "RT1_PROVIDER_UNAVAILABLE"))?;
    let mut engine = super::engine_bootstrap::assemble(providers, true)
        .map_err(|_| error_response(503, "RT1_ENGINE_UNAVAILABLE"))?;
    if !matches!(restore_reviewed_persona(&mut engine), Ok(true)) {
        return Err(error_response(409, "RT1_PERSONA_NOT_REVIEWED"));
    }
    let manager = Rt1OwnerController::new().visitor_sessions();
    let started = if std::env::var("VPR_RT1_DIAGNOSTIC_RTP").as_deref() == Ok("true") {
        Rt1VisitorConversation::start_diagnostic(manager, body.visitor_token, engine, true)
    } else {
        Rt1VisitorConversation::start(manager, body.visitor_token, engine, true)
    };
    let (conversation, bundle) =
        started.map_err(|_| error_response(409, "RT1_VISITOR_START_DENIED"))?;
    // Register before checking revocation: if STOP is uncertain, the
    // quarantined server-side handle must survive even though the browser
    // never received a usable SDP/session ID.
    let conversation = Arc::new(conversation);
    // Capability lookup and the post-registration authority check can invoke
    // remote STOP. Never hold the shared HTTP registry during provider I/O.
    match register_visitor(
        &state.rt1.conversations,
        &session_id,
        &conversation,
        Rt1VisitorConversation::is_active,
    ) {
        Ok(()) => {}
        Err(RegistrationFailure::Collision) => {
            if conversation.close().is_err() {
                return Err(error_response(502, "RT1_REMOTE_STOP_UNCONFIRMED"));
            }
            return Err(error_response(409, "RT1_SESSION_COLLISION"));
        }
        Err(RegistrationFailure::Revoked) => {
            // The browser never receives this session ID, but the server must
            // retain it for STOP reconciliation if the provider is uncertain.
            let receipt_path = conversation.has_remote_video_stop_receipt_path();
            if finalize_registered_stop(
                &state.rt1.conversations,
                &session_id,
                &conversation,
                conversation.close(),
                receipt_path,
            )
            .is_err()
            {
                return Err(error_response(502, "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED"));
            }
            return Err(error_response(409, "RT1_VISITOR_REVOKED_DURING_START"));
        }
    }
    // Authorization can change after registration or during capability
    // discovery. Never lose the retry handle when that late check fails.
    let voice_enabled = conversation.supports_private_voice();
    if !conversation.is_active() {
        let receipt_path = conversation.has_remote_video_stop_receipt_path();
        if finalize_registered_stop(
            &state.rt1.conversations,
            &session_id,
            &conversation,
            conversation.close(),
            receipt_path,
        )
        .is_err()
        {
            return Err(error_response(502, "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED"));
        }
        return Err(error_response(409, "RT1_VISITOR_REVOKED_DURING_START"));
    }
    Ok(json_response(
        200,
        &serde_json::json!({
            "session_id": session_id,
            "signal": bundle,
            "voice_enabled": voice_enabled,
            "diagnostic_rtp": std::env::var("VPR_RT1_DIAGNOSTIC_RTP").as_deref() == Ok("true"),
        }),
    ))
}

fn lookup(state: &AppState, session_id: &str) -> Rt1HttpResultConversation {
    state
        .rt1
        .conversations
        .lock()
        .get(session_id)
        .cloned()
        .ok_or_else(|| error_response(404, "RT1_VISITOR_SESSION_NOT_FOUND"))
}

type Rt1HttpResultConversation = Result<Arc<Rt1VisitorConversation>, HttpResponse>;

fn visitor_answer(request: &mut tiny_http::Request, state: &AppState) -> Rt1HttpResult {
    let body: VisitorAnswer = parse_json(request)?;
    lookup(state, &body.session_id)?
        .submit_answer(&body.sdp)
        .map_err(|code| {
            if code == "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED" {
                error_response(502, "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED")
            } else {
                error_response(409, "RT1_VISITOR_ANSWER_DENIED")
            }
        })?;
    Ok(json_response(200, &serde_json::json!({"ok": true})))
}

fn visitor_ice(request: &mut tiny_http::Request, state: &AppState) -> Rt1HttpResult {
    let body: VisitorIce = parse_json(request)?;
    if body
        .candidate
        .as_ref()
        .is_some_and(|value| value.len() > 4096)
        || body.sdp_mid.as_ref().is_some_and(|value| value.len() > 256)
    {
        return Err(error_response(400, "RT1_INVALID_ICE_CANDIDATE"));
    }
    lookup(state, &body.session_id)?
        .submit_ice(vpr_integration::WebRtcIceCandidate {
            candidate: body.candidate,
            sdp_mid: body.sdp_mid,
            sdp_mline_index: body.sdp_mline_index,
        })
        .map_err(|_| error_response(409, "RT1_VISITOR_ICE_DENIED"))?;
    Ok(json_response(200, &serde_json::json!({"ok": true})))
}

// Text uses the same canonical visitor engine and publication-scoped
// interruptible provider permit as microphone turns. This local diagnostic
// endpoint does not grant authority to the owner engine or public visitors.
fn visitor_text(
    request: &mut tiny_http::Request,
    state: &AppState,
) -> Result<(HttpResponse, Arc<Rt1VisitorConversation>), HttpResponse> {
    let body: VisitorText = parse_json(request)?;
    if body.text.trim().is_empty() || body.text.len() > 8192 {
        return Err(error_response(400, "RT1_INVALID_VISITOR_TEXT"));
    }
    let conversation = lookup(state, &body.session_id)?;
    let result = conversation
        .text_turn(&body.text)
        .map_err(|_| error_response(409, "RT1_VISITOR_TEXT_DENIED"))?;
    // A revoked session may not receive a buffered LLM result even when
    // the provider returns after authority was withdrawn.
    if !conversation.is_active() {
        return Err(error_response(409, "RT1_VISITOR_TEXT_DENIED"));
    }
    Ok((json_response(200, &result), conversation))
}

fn visitor_voice(
    request: &mut tiny_http::Request,
    state: &AppState,
) -> Result<(HttpResponse, Arc<Rt1VisitorConversation>), HttpResponse> {
    if !super::is_json(request) {
        return Err(error_response(415, "JSON_REQUIRED"));
    }
    let raw = super::read_body(request, MAX_RT1_VOICE_JSON_BODY)?;
    let body: VisitorVoice =
        serde_json::from_slice(&raw).map_err(|_| error_response(400, "INVALID_INPUT"))?;
    if body.pcm.is_empty() || body.pcm.len() > MAX_RT1_VOICE_BYTES || body.pcm.len() % 2 != 0 {
        return Err(error_response(400, "RT1_INVALID_MICROPHONE_AUDIO"));
    }
    let conversation = lookup(state, &body.session_id)?;
    let result = conversation
        .voice_turn(body.pcm)
        .map_err(|_| error_response(409, "RT1_VISITOR_VOICE_DENIED"))?;
    if !conversation.is_active() {
        return Err(error_response(409, "RT1_VISITOR_VOICE_DENIED"));
    }
    Ok((json_response(200, &result), conversation))
}

fn visitor_interrupt(request: &mut tiny_http::Request, state: &AppState) -> Rt1HttpResult {
    let body: VisitorSession = parse_json(request)?;
    lookup(state, &body.session_id)?
        .interrupt_turn()
        .map_err(|code| {
            if code == "RT1_VISITOR_INTERRUPT_STOP_UNCONFIRMED" {
                error_response(502, "RT1_VISITOR_INTERRUPT_STOP_UNCONFIRMED")
            } else {
                error_response(409, "RT1_VISITOR_INTERRUPT_DENIED")
            }
        })?;
    // Audio interrupt is not evidence of remote video shutdown.
    Ok(json_response(
        200,
        &serde_json::json!({
            "ok": true, "remote_video_stop_confirmed": false
        }),
    ))
}

fn visitor_status(request: &mut tiny_http::Request, state: &AppState) -> Rt1HttpResult {
    let body: VisitorSession = parse_json(request)?;
    let active = lookup(state, &body.session_id)?.is_active();
    Ok(json_response(200, &serde_json::json!({"active": active})))
}

fn visitor_close(request: &mut tiny_http::Request, state: &AppState) -> Rt1HttpResult {
    let body: VisitorSession = parse_json(request)?;
    // Do not erase a session until an actual remote STOP is confirmed.
    // Keep it for a retry if the network is unreachable.
    let conversation = lookup(state, &body.session_id)?;
    let remote_stop_confirmed = conversation.has_remote_video_stop_receipt_path();
    // The legacy D-ID DELETE and private Echo disconnect are not independent
    // evidence that BOTH remote RTP senders (audio and video) terminated.
    // Preserve the retry handle until the durable worker confirms both.
    finalize_registered_stop(
        &state.rt1.conversations,
        &body.session_id,
        &conversation,
        conversation.close(),
        remote_stop_confirmed,
    )
    .map_err(|code| {
        if code == "RT1_REMOTE_VIDEO_STOP_UNCONFIRMED" {
            error_response(502, "RT1_REMOTE_VIDEO_STOP_UNCONFIRMED")
        } else {
            error_response(502, "RT1_REMOTE_STOP_UNCONFIRMED")
        }
    })?;
    Ok(json_response(
        200,
        &serde_json::json!({
            "ok": true,
            "local_stop_confirmed": true,
            "remote_stop_confirmed": remote_stop_confirmed,
            "remote_video_stop_confirmed": remote_stop_confirmed,
        }),
    ))
}

#[cfg(test)]
#[path = "http_rt1_registration_tests.rs"]
mod registration_tests;
