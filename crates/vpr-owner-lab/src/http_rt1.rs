//! Local-only, explicitly enabled RT1 owner-to-visitor feasibility route.
//! The existing Owner Lab remains bound to 127.0.0.1 and must NEVER be
//! exposed via a public proxy: legacy RT0 routes use localhost CSRF, not
//! independent owner authentication. RT1 ReleaseSpec still blocks deployment.
use std::env;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use tiny_http::Request;
use vpr_owner_lab::{
    LabModalityState, OwnerContextState, OwnerLabStartRequest, ParticipantRole,
    Rt1WorkspaceService,
};
use vpr_policy::publication_lifecycle::VisitorPermit;

use crate::{
    AppState, HttpResponse, error_response, header_value, http_evidence,
    http_session, http_text, json_response, parse_empty_json, parse_json,
};

#[derive(Clone)]
pub(super) struct Rt1VisitorSession {
    pub sequence: u64,
    pub permit: VisitorPermit,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishBody {
    expected_persona_version: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VisitorStartBody {
    consent: bool,
}

/// The flag grants ONLY a loopback feasibility flow using an explicit
/// single-writer Persona file. It cannot open a public listener.
pub(super) fn prepare(rt0_evidence_mode: bool) -> Result<Option<Rt1WorkspaceService>, String> {
    if env::var("VPR_RT1_PRIVATE_PREVIEW").as_deref() != Ok("true") {
        return Ok(None);
    }
    if rt0_evidence_mode {
        return Err("RT1 private preview cannot run during RT0 evidence".into());
    }
    if env::var_os("VPR_OWNER_LAB_PERSONA_STORE_PATH").is_none() {
        return Err("RT1 private preview requires explicit single-writer Persona file".into());
    }
    let service = Rt1WorkspaceService::new();
    service.recover_before_listener()?;
    Ok(Some(service))
}

pub(super) fn route_post(
    path: &str,
    request: &mut Request,
    state: &Arc<AppState>,
) -> Option<Result<HttpResponse, HttpResponse>> {
    if !path.starts_with("/api/rt1/private-preview/") {
        return None;
    }
    let Some(workspace) = state.rt1_workspace.as_ref() else {
        return Some(Err(error_response(403, "RT1_RELEASE_GATE_CLOSED")));
    };
    let result = match path {
        "/api/rt1/private-preview/owner/provision" => {
            parse_empty_json(request).and_then(|()| {
                workspace.provision_owner()
                    .map(|(owner_id, owner_token)| {
                        json_response(201, &serde_json::json!({
                            "owner_id": owner_id, "owner_token": owner_token,
                            "scope": "loopback_feasibility_only"
                        }))
                    })
                    .map_err(|_| error_response(409, "OWNER_PROVISION_DENIED"))
            })
        }
        "/api/rt1/private-preview/owner/status" => {
            owner_bearer(request).and_then(|token| {
                parse_empty_json(request)?;
                workspace.owner_status(token)
                    .map(|status| json_response(200, &status))
                    .map_err(|_| error_response(403, "AUTH_SCOPE_DENIED"))
            })
        }
        "/api/rt1/private-preview/owner/publish" => {
            owner_bearer(request).and_then(|token| {
                let body: PublishBody = parse_json(request)?;
                let approved = {
                    let engine = state.engine.lock()
                        .map_err(|_| error_response(500, "INTERNAL_ERROR"))?;
                    let status = engine.status();
                    status.owner_context_state == OwnerContextState::Reviewed
                        && status.egress_enabled
                        && matches!(status.modality_readiness.text, LabModalityState::Ready)
                        && matches!(status.modality_readiness.voice, LabModalityState::Ready)
                        && matches!(status.modality_readiness.video, LabModalityState::Ready)
                        && matches!(status.session_state.as_str(), "none" | "closed")
                } && state.reference_intake.has_both_references();
                if !approved {
                    return Err(error_response(409, "PREPARATION_OR_CONSENT_REQUIRED"));
                }
                workspace.publish(token, body.expected_persona_version, approved)
                    .map(|visitor_token| json_response(200, &serde_json::json!({
                        "visitor_token": visitor_token,
                        "scope": "loopback_feasibility_only",
                        "public_url": null
                    })))
                    .map_err(|_| error_response(409, "PUBLICATION_DENIED"))
            })
        }
        "/api/rt1/private-preview/owner/pause"
        | "/api/rt1/private-preview/owner/unpublish" => {
            owner_bearer(request).and_then(|token| {
                parse_empty_json(request)?;
                // Start/close use this same replay lock. Publication revoke
                // commits before the registered provider authority is fenced.
                let mut replay = state.replay_source.lock();
                let result = if path.ends_with("/pause") {
                    workspace.pause(token)
                } else {
                    workspace.unpublish(token)
                };
                result.map_err(|_| error_response(403, "PUBLICATION_REVOKE_DENIED"))?;
                *replay = None;
                fence_active_visitor(state);
                Ok(json_response(200, &serde_json::json!({
                    "ok": true, "visitor_authority": "revoked"
                })))
            })
        }
        "/api/rt1/private-preview/visitor/start" => {
            visitor_bearer(request).and_then(|token| {
                let body: VisitorStartBody = parse_json(request)?;
                if !body.consent {
                    return Err(error_response(403, "CONSENT_REQUIRED"));
                }
                http_evidence::ensure_previous_exported(
                    &state.engine, &state.evidence, &state.evidence_export,
                )
                .map_err(|error| error_response(error.status(), error.code()))?;
                http_session::reject_if_session_ending(state)?;
                let mut replay = state.replay_source.lock();
                let permit = workspace.visitor_permit(token)
                    .map_err(|_| error_response(403, "AUTH_SCOPE_DENIED"))?;
                workspace.authorize_visitor(token, &permit, now_millis()?)
                    .map_err(|_| error_response(403, "AUTH_SCOPE_DENIED"))?;
                let mut engine = state.engine.lock()
                    .map_err(|_| error_response(500, "INTERNAL_ERROR"))?;
                let bundle = engine.start_visitor(OwnerLabStartRequest { consent: true })
                    .map_err(|error| crate::lab_error_response(&error))?;
                http_session::register_started_session(state, &mut engine)
                    .map_err(|error| crate::lab_error_response(&error))?;
                *replay = None;
                state.evidence.lock().begin_session(
                    bundle.evidence_session_sequence, ParticipantRole::Visitor,
                ).map_err(|_| error_response(500, "INTERNAL_ERROR"))?;
                state.active_session_sequence.store(
                    bundle.evidence_session_sequence, Ordering::Release,
                );
                state.greeting_session_sequence.store(0, Ordering::Release);
                state.voice_streams.clear();
                *state.rt1_visitor.lock() = Some(Rt1VisitorSession {
                    sequence: bundle.evidence_session_sequence, permit,
                });
                Ok(json_response(200, &bundle))
            })
        }
        "/api/rt1/private-preview/visitor/text" => {
            visitor_bearer(request).and_then(|token| {
                http_session::reject_if_session_ending(state)?;
                let holder = state.rt1_visitor.lock().clone()
                    .ok_or_else(|| error_response(403, "AUTH_SCOPE_DENIED"))?;
                if state.active_session_sequence.load(Ordering::Acquire) != holder.sequence {
                    return Err(error_response(403, "AUTH_SCOPE_DENIED"));
                }
                workspace.authorize_visitor(token, &holder.permit, now_millis()?)
                    .map_err(|_| error_response(403, "AUTH_SCOPE_DENIED"))?;
                http_text::text_turn_response(request, state)
            })
        }
        _ => Err(error_response(404, "NOT_FOUND")),
    };
    Some(result)
}

fn owner_bearer(request: &Request) -> Result<&str, HttpResponse> {
    header_value(request, "X-VPR-Owner-Token")
        .filter(|token| token.len() == 64)
        .ok_or_else(|| error_response(403, "AUTH_SCOPE_DENIED"))
}

fn visitor_bearer(request: &Request) -> Result<&str, HttpResponse> {
    header_value(request, "Authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| token.len() == 64)
        .ok_or_else(|| error_response(403, "AUTH_SCOPE_DENIED"))
}

fn now_millis() -> Result<u64, HttpResponse> {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| error_response(500, "INTERNAL_ERROR"))?;
    u64::try_from(since.as_millis())
        .map_err(|_| error_response(500, "INTERNAL_ERROR"))
}

fn fence_active_visitor(state: &AppState) {
    let Some(visitor) = state.rt1_visitor.lock().take() else {
        return;
    };
    if state.active_session_sequence.load(Ordering::Acquire) != visitor.sequence {
        return;
    }
    state.session_end_requested.store(true, Ordering::Release);
    if let Some(handle) = state.session_revocation.lock().clone() {
        let _ = handle.revoke_authority();
    }
    state.voice_cancel_requested.store(true, Ordering::Release);
    if let Some(handle) = state.active_voice_interrupt.lock().clone() {
        let _ = handle.interrupt();
    }
    if let Some(handle) = state.backend_stop.lock().clone() {
        let _ = handle.stop();
    }
}
