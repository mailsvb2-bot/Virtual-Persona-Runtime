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
    let response = match path {
        "/api/rt1/owner/provision" => {
            let _: serde_json::Value = match parse_json(request) {
                Ok(body) => body,
                Err(response) => return Some(response),
            };
            let controller = Rt1OwnerController::new();
            match controller.provision_owner() {
                Ok((owner_id, owner_token)) => json_response(200, &serde_json::json!({
                    "owner_id": owner_id,
                    "owner_token": owner_token,
                })),
                Err(_) => error_response(409, "RT1_OWNER_PROVISION_DENIED"),
            }
        }
        "/api/rt1/owner/publish" => {
            let body: OwnerPublish = match parse_json(request) {
                Ok(body) => body,
                Err(response) => return Some(response),
            };
            let controller = Rt1OwnerController::new();
            // Readiness is always decided by the server, not a browser flag.
            let readiness = || -> Result<(), String> {
                let engine = state.engine.lock().map_err(|_| "RT1_ENGINE_UNAVAILABLE")?;
                let status = engine.status();
                if !status.egress_enabled || status.owner_context_state != OwnerContextState::Reviewed {
                    return Err("RT1_REVIEWED_PERSONA_AND_EGRESS_REQUIRED".into());
                }
                for name in ["RT1_WORKER_ENDPOINT", "RT1_WORKER_TOKEN"] {
                    if std::env::var(name).ok().is_none_or(|value| value.trim().is_empty()) {
                        return Err("RT1_WORKER_NOT_CONFIGURED".into());
                    }
                }
                Ok(())
            };
            match controller.publish(
                &body.owner_token,
                body.expected_persona_version,
                readiness,
            ) {
                Ok(visitor_token) => json_response(200, &serde_json::json!({
                    "visitor_token": visitor_token,
                })),
                Err(_) => error_response(409, "RT1_PUBLISH_DENIED"),
            }
        }
        "/api/rt1/owner/pause" | "/api/rt1/owner/unpublish" => {
            let body: OwnerToken = match parse_json(request) {
                Ok(body) => body,
                Err(response) => return Some(response),
            };
            let controller = Rt1OwnerController::new();
            let result = if path.ends_with("/pause") {
                controller.pause(&body.owner_token)
            } else {
                controller.unpublish(&body.owner_token)
            };
            match result {
                Ok(()) => json_response(200, &serde_json::json!({
                    "ok": true,
                    "publication_revoked": true,
                    "remote_stop_confirmed": false,
                })),
                Err(_) => error_response(403, "RT1_OWNER_ACTION_DENIED"),
            }
        }
        "/api/rt1/visitor/start" => {
            let body: VisitorStart = match parse_json(request) {
                Ok(body) => body,
                Err(response) => return Some(response),
            };
            if !body.consent {
                return Some(error_response(403, "CONSENT_REQUIRED"));
            }
            // Distinct visitor-scoped engine. Never share the owner's engine.
            let providers = match ProviderBundle::from_env(false) {
                Ok(providers) => providers,
                Err(_) => return Some(error_response(503, "RT1_PROVIDER_UNAVAILABLE")),
            };
            let mut engine = match super::engine_bootstrap::assemble(providers, true) {
                Ok(engine) => engine,
                Err(_) => return Some(error_response(503, "RT1_ENGINE_UNAVAILABLE")),
            };
            match restore_reviewed_persona(&mut engine) {
                Ok(true) => {}
                _ => return Some(error_response(409, "RT1_PERSONA_NOT_REVIEWED")),
            };
            let manager = Rt1OwnerController::new().visitor_sessions();
            match Rt1VisitorConversation::start(manager, body.visitor_token, engine, true) {
                Ok((conversation, bundle)) => {
                    let mut raw = [0_u8; 32];
                    if getrandom::fill(&mut raw).is_err() {
                        let _ = conversation.close();
                        return Some(error_response(503, "RT1_SESSION_ENTROPY_UNAVAILABLE"));
                    }
                    let mut session_id = String::with_capacity(64);
                    for octet in raw {
                        use std::fmt::Write;
                        if write!(&mut session_id, "{octet:02x}").is_err() {
                            let _ = conversation.close();
                            return Some(error_response(500, "RT1_SESSION_ID_FAILURE"));
                        }
                    }
                    let mut active = state.rt1.conversations.lock();
                    if active.contains_key(&session_id) {
                        drop(active);
                        let _ = conversation.close();
                        return Some(error_response(409, "RT1_SESSION_COLLISION"));
                    }
                    active.insert(session_id.clone(), Arc::new(conversation));
                    json_response(200, &serde_json::json!({
                        "session_id": session_id,
                        "signal": bundle,
                    }))
                }
                Err(_) => error_response(409, "RT1_VISITOR_START_DENIED"),
            }
        }
        "/api/rt1/visitor/answer" => {
            let body: VisitorAnswer = match parse_json(request) {
                Ok(body) => body,
                Err(response) => return Some(response),
            };
            let conversation = state.rt1.conversations.lock().get(&body.session_id).cloned();
            match conversation {
                Some(conversation) if conversation.submit_answer(&body.sdp).is_ok() => {
                    json_response(200, &serde_json::json!({"ok":true}))
                }
                _ => error_response(409, "RT1_VISITOR_ANSWER_DENIED"),
            }
        }
        "/api/rt1/visitor/status" => {
            let body: VisitorSession = match parse_json(request) {
                Ok(body) => body,
                Err(response) => return Some(response),
            };
            let conversation = state.rt1.conversations.lock().get(&body.session_id).cloned();
            match conversation {
                Some(conversation) => json_response(200, &serde_json::json!({
                    "active": conversation.is_active(),
                })),
                None => error_response(404, "RT1_VISITOR_SESSION_NOT_FOUND"),
            }
        }
        "/api/rt1/visitor/close" => {
            let body: VisitorSession = match parse_json(request) {
                Ok(body) => body,
                Err(response) => return Some(response),
            };
            let conversation = state.rt1.conversations.lock().get(&body.session_id).cloned();
            match conversation {
                Some(conversation) if conversation.close().is_ok() => {
                    state.rt1.conversations.lock().remove(&body.session_id);
                    json_response(200, &serde_json::json!({
                        "ok":true,
                        "remote_stop_confirmed":true,
                    }))
                }
                Some(_) => error_response(502, "RT1_REMOTE_STOP_UNCONFIRMED"),
                None => error_response(404, "RT1_VISITOR_SESSION_NOT_FOUND"),
            }
        }
        _ => return None,
    };
    Some(response)
}
