//! Local-experiment-only binding of the canonical STT/LLM/TTS avatar engine
//! to the already durable worker's authenticated dual-track STOP operation.
use std::sync::Arc;
use std::time::Duration;

use vpr_owner_lab::{Rt1OwnerController, Rt1RemoteMediaRevocation};
use vpr_provider_local_open_source::{
    OperationWorkerAvatar, operation_client::RemoteOperationClient,
};

use super::{HttpResponse, error_response};

pub(super) fn selected() -> bool {
    std::env::var("VPR_RT1_CANONICAL_VOICE_WORKER").as_deref() == Ok("true")
}

/// Build one exact operation identity before provider CREATE. The caller must
/// register its remote STOP handle after visitor authorization and before
/// invoking the canonical avatar engine's existing startup path.
pub(super) fn prepare(
    operation_id: &str,
) -> Result<(Arc<Rt1RemoteMediaRevocation>, Box<OperationWorkerAvatar>), HttpResponse> {
    let endpoint = std::env::var("RT1_WORKER_ENDPOINT")
        .map_err(|_| error_response(503, "RT1_WORKER_NOT_CONFIGURED"))?;
    let token = std::env::var("RT1_WORKER_TOKEN")
        .map_err(|_| error_response(503, "RT1_WORKER_NOT_CONFIGURED"))?;
    // Bind this exact worker client to the current canonical reviewed
    // Persona BEFORE any remote readiness or CREATE. The same identity is
    // rechecked by the worker at CREATE, not trusted from the browser.
    let (persona_id, persona_version) = Rt1OwnerController::new()
        .current_persona_identity()
        .map_err(|_| error_response(409, "RT1_PERSONA_NOT_REVIEWED"))?;
    prepare_bound(
        operation_id,
        &endpoint,
        &token,
        &persona_id,
        persona_version,
    )
}

/// Construct exactly the same authenticated worker/STOP adapter, but bind its
/// Persona and credentials to an authenticated owner rather than a global
/// environment variable. Never call this with browser-supplied identities.
pub(super) fn prepare_bound(
    operation_id: &str,
    endpoint: &str,
    token: &str,
    persona_id: &str,
    persona_version: u64,
) -> Result<(Arc<Rt1RemoteMediaRevocation>, Box<OperationWorkerAvatar>), HttpResponse> {
    let client = RemoteOperationClient::new(endpoint, token, Duration::from_secs(5))
        .and_then(|client| client.bind_persona(persona_id, persona_version))
        .map_err(|_| error_response(503, "RT1_WORKER_CONFIGURATION_INVALID"))?;
    let remote = Arc::new(
        Rt1RemoteMediaRevocation::new(client.clone(), operation_id.to_owned(), 1)
            .map_err(|_| error_response(503, "RT1_WORKER_CONFIGURATION_INVALID"))?,
    );
    let created_owner = Arc::clone(&remote);
    let stop_owner = Arc::clone(&remote);
    let avatar = Box::new(
        OperationWorkerAvatar::with_remote_owner(
            client,
            operation_id.to_owned(),
            1,
            move |session_id| created_owner.record_created_session(session_id),
            move || {
                // The adapter can run while OwnerLabEngine is locked. It may
                // only revoke authority and wake the existing reconciler here;
                // remote STOP itself runs outside that mutex.
                stop_owner.request_revoke();
                if stop_owner.is_stop_confirmed() {
                    Ok(())
                } else {
                    Err("RT1_REMOTE_MEDIA_STOP_UNCONFIRMED".to_owned())
                }
            },
        )
        .map_err(|_| error_response(503, "RT1_WORKER_CONFIGURATION_INVALID"))?,
    );
    Ok((remote, avatar))
}
