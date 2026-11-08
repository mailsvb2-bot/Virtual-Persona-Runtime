//! Local-experiment-only binding of the canonical STT/LLM/TTS avatar engine
//! to the already durable worker's authenticated dual-track STOP operation.
use std::sync::Arc;
use std::time::Duration;

use vpr_owner_lab::Rt1RemoteMediaRevocation;
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
    let client = RemoteOperationClient::new(&endpoint, &token, Duration::from_secs(5))
        .map_err(|_| error_response(503, "RT1_WORKER_CONFIGURATION_INVALID"))?;
    let remote = Arc::new(
        Rt1RemoteMediaRevocation::new(client.clone(), operation_id.to_owned(), 1)
            .map_err(|_| error_response(503, "RT1_WORKER_CONFIGURATION_INVALID"))?,
    );
    let avatar = Box::new(
        OperationWorkerAvatar::new(client, operation_id.to_owned(), 1)
            .map_err(|_| error_response(503, "RT1_WORKER_CONFIGURATION_INVALID"))?,
    );
    Ok((remote, avatar))
}
