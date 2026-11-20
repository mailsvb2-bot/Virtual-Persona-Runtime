//! Existing RT1 HTTP visitor start: one reviewed Persona, one canonical
//! owner visitor authority and a worker STOP handle registered before CREATE.
use super::{
    AppState, HttpResponse, ProviderBundle, RegistrationFailure, Rt1OwnerController,
    Rt1RemoteMediaRevocation, Rt1VisitorConversation, VisitorStart, error_response,
    finalize_registered_stop, json_response, new_visitor_session_id, parse_json, register_visitor,
    restore_reviewed_persona_for_visitor, startup_errors, verified_worker,
};
use std::sync::Arc;

fn selected_provider_bundle(
    session_id: &str,
) -> Result<(ProviderBundle, Option<Arc<Rt1RemoteMediaRevocation>>), HttpResponse> {
    if verified_worker::selected() {
        let (remote, avatar) = verified_worker::prepare(session_id)?;
        let providers = ProviderBundle::from_env_with_avatar(true, avatar)
            .map_err(|_| error_response(503, "RT1_PROVIDER_UNAVAILABLE"))?;
        // The worker accepts PCM, not provider text. Without canonical TTS
        // the browser must not receive a voice-enabled session.
        if providers.tts.is_none() {
            return Err(error_response(503, "RT1_TTS_NOT_CONFIGURED"));
        }
        Ok((providers, Some(remote)))
    } else {
        let providers = ProviderBundle::from_env(false)
            .map_err(|_| error_response(503, "RT1_PROVIDER_UNAVAILABLE"))?;
        Ok((providers, None))
    }
}

// Pre-provider collision rejection, including reserved IDs and durable
// unresolved STOP targets. The lease spans the entire START and drops safely
// on every error; it never removes a remote cleanup target.
fn reserve_session_identity(
    state: &AppState,
) -> Result<(String, super::PendingStart<'_>), HttpResponse> {
    for _ in 0..8 {
        let candidate = new_visitor_session_id()?;
        if let Some(lease) = super::reserve_start(&state.rt1, candidate.clone()) {
            return Ok((candidate, lease));
        }
    }
    Err(error_response(503, "RT1_SESSION_ID_RESERVATION_FAILED"))
}

/// Publication must only issue visitor tokens when the same authorized
/// transport can subsequently start and supply auditable audio/video STOP.
/// D-ID V2 `LiveKit` currently lacks a confirmed remote video STOP path.
pub(super) fn publication_media_transport_ready(
    verified_worker: bool,
    diagnostic_rtp: bool,
    worker_endpoint: Option<&str>,
    worker_token: Option<&str>,
) -> Result<(), &'static str> {
    // Diagnostic aiortc tracks prove transport only, never Persona video.
    // They may remain available to local sender tests but cannot authorize a
    // public visitor token, even when worker credentials are configured.
    if diagnostic_rtp {
        return Err("RT1_VIDEO_RENDERER_UNAVAILABLE");
    }
    if !verified_worker {
        return Err("RT1_REMOTE_VIDEO_STOP_UNCONFIRMED");
    }
    if worker_endpoint.is_none_or(|value| value.trim().is_empty())
        || worker_token.is_none_or(|value| value.trim().is_empty())
    {
        return Err("RT1_WORKER_NOT_CONFIGURED");
    }
    Ok(())
}

/// Validate the canonical worker voice composition BEFORE publishing a
/// token, using the same provider constructor as visitor START. This is a
/// configuration preflight only: it never creates or starts remote media.
/// Diagnostic RTP remains a local transport fixture, never a public visitor route.
pub(super) fn publication_voice_provider_ready() -> Result<(), &'static str> {
    if !verified_worker::selected() {
        return Ok(());
    }
    let (remote, avatar) = verified_worker::prepare("rt1-publication-preflight")
        .map_err(|_| "RT1_WORKER_NOT_CONFIGURED")?;
    let providers = ProviderBundle::from_env_with_avatar(true, avatar)
        .map_err(|_| "RT1_PROVIDER_UNAVAILABLE")?;
    if providers.tts.is_none() {
        return Err("RT1_TTS_NOT_CONFIGURED");
    }
    // Only an authenticated ledger admission snapshot is meaningful.
    // A public /healthz result does not prove token validity or safe CREATE.
    remote.publication_ready()
}

// This is only a capability binding on the existing engine. No new
// session, provider or HTTP router is created by the PostgreSQL extension.
fn bind_optional_memory_on_start(
    engine: &mut vpr_owner_lab::OwnerLabEngine,
    token: Option<&str>,
) -> Result<(), HttpResponse> {
    #[cfg(not(feature = "rt1-memory-postgres"))]
    {
        let _ = engine;
        if token.is_some() {
            return Err(error_response(503, "RT1_MEMORY_DISABLED"));
        }
    }
    #[cfg(feature = "rt1-memory-postgres")]
    if let Some(memory_token) = token {
        super::memory::bind_on_start(engine, memory_token)
            .map_err(|_| error_response(403, "RT1_MEMORY_BIND_DENIED"))?;
    }
    Ok(())
}

// A read-only canonical admission before provider/engine allocation. The
// returned manager is reused by the required second admission at START.
fn admitted_visitor_manager(
    token: &str,
) -> Result<Arc<vpr_owner_lab::VisitorSessionManager>, HttpResponse> {
    let manager = Rt1OwnerController::new().visitor_sessions();
    manager
        .preflight_visitor(token)
        .map_err(|_| error_response(409, "RT1_VISITOR_ACCESS_DENIED"))?;
    Ok(manager)
}

pub(super) fn visitor_start(
    request: &mut tiny_http::Request,
    state: &AppState,
) -> Result<(HttpResponse, Arc<Rt1VisitorConversation>), HttpResponse> {
    let body: VisitorStart = parse_json(request)?;
    if !body.consent {
        return Err(error_response(403, "CONSENT_REQUIRED"));
    }
    let manager = admitted_visitor_manager(&body.visitor_token)?;
    // Additional accounts cannot reuse the primary Owner Lab's credentials.
    // Check before constructing adapters or allocating any remote media.
    Rt1OwnerController::new()
        .require_legacy_provider_owner(&body.visitor_token)
        .map_err(|_| error_response(503, "RT1_OWNER_PROVIDER_BINDING_REQUIRED"))?;
    let (session_id, _reservation) = reserve_session_identity(state)?;
    let (providers, worker) = selected_provider_bundle(&session_id)?;
    let mut engine = super::super::engine_bootstrap::assemble(providers, true)
        .map_err(|_| error_response(503, "RT1_ENGINE_UNAVAILABLE"))?;
    restore_reviewed_persona_for_visitor(&mut engine, &body.visitor_token)
        .map_err(|_| error_response(409, "RT1_PERSONA_NOT_REVIEWED"))?;
    bind_optional_memory_on_start(&mut engine, body.memory_token.as_deref())?;
    let started = if let Some(worker) = worker {
        let retry_id = session_id.clone();
        Rt1VisitorConversation::start_with_verified_worker(
            manager,
            body.visitor_token,
            engine,
            true,
            worker,
            |remote| {
                state
                    .rt1
                    .register_remote(retry_id.clone(), Arc::clone(remote))
            },
        )
    } else if std::env::var("VPR_RT1_DIAGNOSTIC_RTP").as_deref() == Ok("true") {
        let retry_id = session_id.clone();
        Rt1VisitorConversation::start_diagnostic(
            manager,
            body.visitor_token,
            engine,
            true,
            |remote| state.rt1.register_remote(retry_id, remote),
        )
    } else {
        // The unverified D-ID route is deliberately never promoted.
        return Err(error_response(503, "RT1_REMOTE_VIDEO_STOP_UNCONFIRMED"));
    };
    let (conversation, bundle) =
        started.map_err(|reason| startup_errors::startup_error_response(&reason))?;
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
    Ok((
        json_response(
            200,
            &serde_json::json!({
                "session_id": session_id,
                "signal": bundle,
                "voice_enabled": voice_enabled,
                // The existing aiortc operation worker performs non-trickle ICE:
                // candidates are embedded in SDP, and its separate ICE endpoint
                // is intentionally denied. Preserve the browser's non-trickle
                // signaling mode for both worker and diagnostic RTP paths.
                "diagnostic_rtp": verified_worker::selected() || std::env::var("VPR_RT1_DIAGNOSTIC_RTP").as_deref() == Ok("true"),
            }),
        ),
        conversation,
    ))
}

pub(super) fn validate_legacy_owner_publication(
    owner_token: &str,
    expected_version: u64,
) -> Result<vpr_owner_lab::WorkspaceStatus, HttpResponse> {
    // Resolve the owner solely from the canonical credential; this global
    // provider stack is not safe for an additional owner's Persona.
    let controller = Rt1OwnerController::new();
    let owner = controller
        .status(owner_token)
        .map_err(|_| error_response(403, "RT1_OWNER_ACTION_DENIED"))?;
    if owner.persona_version != expected_version {
        return Err(error_response(409, "RT1_STALE_PERSONA_VERSION"));
    }
    let primary = controller
        .current_persona_identity()
        .map_err(|_| error_response(503, "RT1_OWNER_ENGINE_UNAVAILABLE"))?;
    if (owner.persona_id.as_str(), owner.persona_version) != (primary.0.as_str(), primary.1) {
        return Err(error_response(503, "RT1_OWNER_PROVIDER_BINDING_REQUIRED"));
    }
    Ok(owner)
}
