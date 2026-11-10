//! Localhost-only RT1 browser entry. Uses the existing owner workspace and
//! visitor conversation, never the owner RT0 engine as a visitor's authority.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::{Condvar, Mutex};
use serde::Deserialize;
use vpr_owner_lab::{
    OwnerContextState, ProviderBundle, Rt1OwnerController, Rt1RemoteMediaRevocation,
    Rt1VisitorConversation, restore_reviewed_persona,
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
    // An identity must be reserved BEFORE provider CREATE; collisions are not
    // repaired by overwriting an existing remote STOP retry handle.
    pending_starts: Mutex<HashSet<String>>,
    // Retained BEFORE remote CREATE, even if no browser session ID is returned.
    // Never hold this registry lock while doing provider network I/O.
    remote_cleanup: Mutex<HashMap<String, Arc<Rt1RemoteMediaRevocation>>>,
    cleanup_notify: Arc<(Mutex<u64>, Condvar)>,
}

#[path = "http_rt1_cleanup.rs"]
mod cleanup;
pub(super) use cleanup::spawn_cleanup_reconciler;
#[path = "http_rt1_start.rs"]
mod start;
#[path = "http_rt1_verified_worker.rs"]
mod verified_worker;
use start::visitor_start;

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
        // START may allocate a sender; ANSWER may activate its audio/video
        // tracks. Losing either final HTTP acknowledgement is ambiguous and
        // must revoke the exact sender through the existing STOP reconciler.
        let must_compensate = path == "/api/rt1/visitor/start"
            || path == "/api/rt1/visitor/answer"
            || path == "/api/rt1/visitor/ice";
        if let Ok(_lease) = conversation.begin_browser_delivery() {
            if request.respond(routed.response).is_err() && must_compensate {
                conversation.revoke_undelivered_start();
            }
        } else {
            if must_compensate {
                conversation.revoke_undelivered_start();
            }
            let _ = request.respond(error_response(409, "RT1_VISITOR_DELIVERY_DENIED"));
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
    if path == "/api/rt1/visitor/start" {
        // SDP/ICE credentials are browser egress too: a revoked owner must
        // not receive a new sender offer after the final admission check.
        return Some(guarded_reply(visitor_start(request, state)));
    }
    if path == "/api/rt1/visitor/text" {
        return Some(guarded_reply(visitor_text(request, state)));
    }
    if path == "/api/rt1/visitor/voice" {
        return Some(guarded_reply(visitor_voice(request, state)));
    }
    if path == "/api/rt1/visitor/answer" {
        // Successful signaling may start remote media: retain the browser
        // delivery permit through the acknowledgement write.
        return Some(guarded_reply(visitor_answer(request, state)));
    }
    if path == "/api/rt1/visitor/ice" {
        return Some(guarded_reply(visitor_ice(request, state)));
    }
    let result = match path {
        "/api/rt1/owner/provision" => owner_provision(request),
        "/api/rt1/owner/publish" => owner_publish(request, state),
        "/api/rt1/owner/pause" => owner_revoke(request, false),
        "/api/rt1/owner/unpublish" => owner_revoke(request, true),
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
        if verified_worker::selected()
            || std::env::var("VPR_RT1_DIAGNOSTIC_RTP").as_deref() == Ok("true")
        {
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

/// A START holds this reservation until registration or failure. The drop
/// guard never certifies remote STOP and cannot clear a durable cleanup handle.
struct PendingStart<'a> {
    pending: &'a Mutex<HashSet<String>>,
    id: String,
}

impl Drop for PendingStart<'_> {
    fn drop(&mut self) {
        self.pending.lock().remove(&self.id);
    }
}

fn reserve_start(state: &Rt1HttpState, id: String) -> Option<PendingStart<'_>> {
    // A pending ID, an active/retryable visitor, or a durable unresolved
    // remote STOP target must never be handed to a second provider CREATE.
    // Hold pending across both checks so concurrent START cannot pass twice.
    let mut pending = state.pending_starts.lock();
    if pending.contains(&id)
        || state.conversations.lock().contains_key(&id)
        || state.remote_cleanup.lock().contains_key(&id)
    {
        return None;
    }
    pending.insert(id.clone());
    Some(PendingStart {
        pending: &state.pending_starts,
        id,
    })
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

fn visitor_answer(
    request: &mut tiny_http::Request,
    state: &AppState,
) -> Result<(HttpResponse, Arc<Rt1VisitorConversation>), HttpResponse> {
    let body: VisitorAnswer = parse_json(request)?;
    let conversation = lookup(state, &body.session_id)?;
    conversation.submit_answer(&body.sdp).map_err(|code| {
        if code == "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED" {
            error_response(502, "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED")
        } else {
            error_response(409, "RT1_VISITOR_ANSWER_DENIED")
        }
    })?;
    Ok((
        json_response(200, &serde_json::json!({"ok": true})),
        conversation,
    ))
}

fn visitor_ice(
    request: &mut tiny_http::Request,
    state: &AppState,
) -> Result<(HttpResponse, Arc<Rt1VisitorConversation>), HttpResponse> {
    let body: VisitorIce = parse_json(request)?;
    if body
        .candidate
        .as_ref()
        .is_some_and(|value| value.len() > 4096)
        || body.sdp_mid.as_ref().is_some_and(|value| value.len() > 256)
    {
        return Err(error_response(400, "RT1_INVALID_ICE_CANDIDATE"));
    }
    let conversation = lookup(state, &body.session_id)?;
    conversation
        .submit_ice(vpr_integration::WebRtcIceCandidate {
            candidate: body.candidate,
            sdp_mid: body.sdp_mid,
            sdp_mline_index: body.sdp_mline_index,
        })
        .map_err(|_| error_response(409, "RT1_VISITOR_ICE_DENIED"))?;
    Ok((
        json_response(200, &serde_json::json!({"ok": true})),
        conversation,
    ))
}

// Canonical visitor permit; no public RT1 provider egress.
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
    let conversation = lookup(state, &body.session_id)?;
    conversation.interrupt_turn().map_err(|code| {
        if code == "RT1_VISITOR_INTERRUPT_STOP_UNCONFIRMED" {
            error_response(502, "RT1_VISITOR_INTERRUPT_STOP_UNCONFIRMED")
        } else {
            error_response(409, "RT1_VISITOR_INTERRUPT_DENIED")
        }
    })?;
    // Only the worker's authenticated BOTH-track receipt may confirm video.
    Ok(json_response(
        200,
        &serde_json::json!({
            "ok": true,
            "remote_video_stop_confirmed": conversation.remote_media_stop_confirmed(),
        }),
    ))
}

fn visitor_status(request: &mut tiny_http::Request, state: &AppState) -> Rt1HttpResult {
    let body: VisitorSession = parse_json(request)?;
    let c = lookup(state, &body.session_id)?;
    // Only an authenticated dual-track worker receipt can confirm remote STOP.
    let stopped = c.remote_media_stop_confirmed();
    let status = serde_json::json!({
        "active": c.is_active(), "remote_stop_confirmed": stopped,
        "remote_video_stop_confirmed": stopped,
    });
    Ok(json_response(200, &status))
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
