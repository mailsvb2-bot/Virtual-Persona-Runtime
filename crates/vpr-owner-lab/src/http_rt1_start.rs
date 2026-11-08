//! Existing RT1 HTTP visitor start: one reviewed Persona, one canonical
//! owner visitor authority and a worker STOP handle registered before CREATE.
use super::*;

pub(super) fn visitor_start(
    request: &mut tiny_http::Request,
    state: &AppState,
) -> Result<(HttpResponse, Arc<Rt1VisitorConversation>), HttpResponse> {
    let body: VisitorStart = parse_json(request)?;
    if !body.consent {
        return Err(error_response(403, "CONSENT_REQUIRED"));
    }
    // Do not call the provider until the retry handle exists.
    let session_id = new_visitor_session_id()?;
    let mut providers = ProviderBundle::from_env(verified_worker::selected())
        .map_err(|_| error_response(503, "RT1_PROVIDER_UNAVAILABLE"))?;
    let worker = verified_worker::prepare(&mut providers, &session_id)?;
    let mut engine = super::super::engine_bootstrap::assemble(providers, true)
        .map_err(|_| error_response(503, "RT1_ENGINE_UNAVAILABLE"))?;
    if !matches!(restore_reviewed_persona(&mut engine), Ok(true)) {
        return Err(error_response(409, "RT1_PERSONA_NOT_REVIEWED"));
    }
    let manager = Rt1OwnerController::new().visitor_sessions();
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
    Ok((
        json_response(
            200,
            &serde_json::json!({
                "session_id": session_id,
                "signal": bundle,
                "voice_enabled": voice_enabled,
                "diagnostic_rtp": verified_worker::selected() || std::env::var("VPR_RT1_DIAGNOSTIC_RTP").as_deref() == Ok("true"),
            }),
        ),
        conversation,
    ))
}
