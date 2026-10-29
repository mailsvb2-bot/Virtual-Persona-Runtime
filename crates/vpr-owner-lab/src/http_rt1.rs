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

pub(super) fn route_post(
    path: &str,
    request: &mut tiny_http::Request,
    state: &Arc<AppState>,
) -> Option<HttpResponse> {
    let result = match path {
        "/api/rt1/owner/provision" => owner_provision(request),
        "/api/rt1/owner/publish" => owner_publish(request, state),
        "/api/rt1/owner/pause" => owner_revoke(request, false),
        "/api/rt1/owner/unpublish" => owner_revoke(request, true),
        "/api/rt1/visitor/start" => visitor_start(request, state),
        "/api/rt1/visitor/answer" => visitor_answer(request, state),
        "/api/rt1/visitor/ice" => visitor_ice(request, state),
        "/api/rt1/visitor/status" => visitor_status(request, state),
        "/api/rt1/visitor/close" => visitor_close(request, state),
        _ => return None,
    };
    Some(result.unwrap_or_else(|response| response))
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

fn visitor_start(request: &mut tiny_http::Request, state: &AppState) -> Rt1HttpResult {
    let body: VisitorStart = parse_json(request)?;
    if !body.consent {
        return Err(error_response(403, "CONSENT_REQUIRED"));
    }
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
    let mut raw = [0_u8; 32];
    if getrandom::fill(&mut raw).is_err() {
        let _ = conversation.close();
        return Err(error_response(503, "RT1_SESSION_ENTROPY_UNAVAILABLE"));
    }
    let mut session_id = String::with_capacity(64);
    for byte in raw {
        session_id.push(char::from(super::HEX[usize::from(byte >> 4)]));
        session_id.push(char::from(super::HEX[usize::from(byte & 0x0f)]));
    }
    let mut active = state.rt1.conversations.lock();
    if active.contains_key(&session_id) {
        drop(active);
        let _ = conversation.close();
        return Err(error_response(409, "RT1_SESSION_COLLISION"));
    }
    active.insert(session_id.clone(), Arc::new(conversation));
    Ok(json_response(
        200,
        &serde_json::json!({
            "session_id": session_id,
            "signal": bundle,
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
        .map_err(|_| error_response(409, "RT1_VISITOR_ANSWER_DENIED"))?;
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

fn visitor_status(request: &mut tiny_http::Request, state: &AppState) -> Rt1HttpResult {
    let body: VisitorSession = parse_json(request)?;
    let active = lookup(state, &body.session_id)?.is_active();
    Ok(json_response(200, &serde_json::json!({"active": active})))
}

fn visitor_close(request: &mut tiny_http::Request, state: &AppState) -> Rt1HttpResult {
    let body: VisitorSession = parse_json(request)?;
    // Do not erase a session until an actual remote STOP is confirmed.
    // Keep it for a retry if the network is unreachable.
    lookup(state, &body.session_id)?
        .close()
        .map_err(|_| error_response(502, "RT1_REMOTE_STOP_UNCONFIRMED"))?;
    state.rt1.conversations.lock().remove(&body.session_id);
    Ok(json_response(
        200,
        &serde_json::json!({
            "ok": true,
            "remote_stop_confirmed": true,
        }),
    ))
}
