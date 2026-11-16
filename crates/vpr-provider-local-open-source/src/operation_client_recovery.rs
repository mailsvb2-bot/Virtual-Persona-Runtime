//! Bounded recovery for unresolved operations in the canonical RT1 worker ledger.
use std::collections::HashSet;
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use vpr_integration::ProviderError;

use super::{Generation, RemoteOperationClient, denied};

#[derive(Debug, Deserialize)]
struct CancelReply {
    operation_id: String,
    session_id: String,
    generation: u64,
    audio_stopped: bool,
    video_stopped: bool,
}

#[derive(Debug, Deserialize)]
struct StopConflictReply {
    error: String,
}

#[derive(Debug, Deserialize)]
struct PendingStopReply {
    operation_id: String,
    #[serde(default)]
    stop_confirmed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnresolvedOperationsReply {
    operations: Vec<UnresolvedOperation>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnresolvedOperation {
    operation_id: String,
    generation: u64,
    phase: String,
}

const STOP_RETRY_WINDOW: Duration = Duration::from_secs(3);
const STOP_RETRY_INITIAL_BACKOFF: Duration = Duration::from_millis(20);
const STOP_RETRY_MAX_BACKOFF: Duration = Duration::from_millis(160);
const MAX_UNRESOLVED_OPERATIONS: usize = 64;
const STARTUP_RECOVERY_WINDOW: Duration = Duration::from_secs(10);

impl RemoteOperationClient {
    /// # Errors
    /// Retries only explicit pending/STOP-lock contention and requires both
    /// remote sender receipts for the exact operation and generation.
    pub fn cancel_confirmed(
        &self,
        operation_id: &str,
        generation: u64,
    ) -> Result<String, ProviderError> {
        cancel_confirmed_until(
            self,
            operation_id,
            generation,
            Instant::now() + STOP_RETRY_WINDOW,
        )
    }

    /// Revoke every unresolved operation recorded by the worker's durable ledger.
    ///
    /// Owner Lab calls this before opening its listener after a process restart.
    /// The worker remains the sole owner of operation identity and STOP receipts;
    /// this method never infers completion from a local socket or process exit.
    ///
    /// # Errors
    /// Fails closed on malformed operation inventory, timeout, or any missing
    /// audio/video STOP receipt. The caller must not open the visitor listener.
    pub fn revoke_unresolved(&self) -> Result<usize, ProviderError> {
        let deadline = Instant::now() + STARTUP_RECOVERY_WINDOW;
        let pending = unresolved_operations_until(self, deadline)?;
        for operation in &pending {
            cancel_confirmed_until(
                self,
                &operation.operation_id,
                operation.generation,
                deadline,
            )?;
        }
        if !unresolved_operations_until(self, deadline)?.is_empty() {
            return Err(denied());
        }
        Ok(pending.len())
    }
}

fn cancel_confirmed_until(
    client: &RemoteOperationClient,
    operation_id: &str,
    generation: u64,
    deadline: Instant,
) -> Result<String, ProviderError> {
    if generation == 0 {
        return Err(denied());
    }
    let url = client.url(operation_id)?;
    let mut backoff = STOP_RETRY_INITIAL_BACKOFF;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(denied());
        }
        let response = client
            .client
            .delete(url.clone())
            .bearer_auth(&client.token)
            .json(&Generation { generation })
            .timeout(remaining)
            .send()
            .map_err(|_| denied())?;
        let status = response.status();
        if status == reqwest::StatusCode::ACCEPTED {
            let pending: PendingStopReply =
                super::super::decode_json_response(response, None).map_err(|_| denied())?;
            if pending.operation_id != operation_id || pending.stop_confirmed {
                return Err(denied());
            }
        } else if status == reqwest::StatusCode::CONFLICT {
            let conflict: StopConflictReply =
                super::super::decode_json_response(response, None).map_err(|_| denied())?;
            if conflict.error != "STOP_IN_PROGRESS" {
                return Err(denied());
            }
        } else {
            if !status.is_success() {
                return Err(denied());
            }
            let reply: CancelReply =
                super::super::decode_json_response(response, None).map_err(|_| denied())?;
            if reply.operation_id != operation_id
                || reply.generation != generation
                || reply.session_id.is_empty()
                || !reply.audio_stopped
                || !reply.video_stopped
            {
                return Err(denied());
            }
            return Ok(reply.session_id);
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(denied());
        }
        thread::sleep(backoff.min(remaining));
        backoff = (backoff * 2).min(STOP_RETRY_MAX_BACKOFF);
    }
}

fn unresolved_operations_until(
    client: &RemoteOperationClient,
    deadline: Instant,
) -> Result<Vec<UnresolvedOperation>, ProviderError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(denied());
    }
    let mut url = client.base.clone();
    url.path_segments_mut()
        .map_err(|()| denied())?
        .extend(["v1", "avatar", "operations"]);
    let response = client
        .client
        .get(url)
        .bearer_auth(&client.token)
        .timeout(remaining)
        .send()
        .map_err(|_| denied())?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(denied());
    }
    let reply: UnresolvedOperationsReply =
        super::super::decode_json_response(response, None).map_err(|_| denied())?;
    if reply.operations.len() > MAX_UNRESOLVED_OPERATIONS {
        return Err(denied());
    }
    let mut identities = HashSet::with_capacity(reply.operations.len());
    for operation in &reply.operations {
        client.url(&operation.operation_id)?;
        if operation.generation == 0
            || !matches!(
                operation.phase.as_str(),
                "creating" | "active" | "stopping" | "stopped" | "uncertain"
            )
            || !identities.insert(operation.operation_id.as_str())
        {
            return Err(denied());
        }
    }
    Ok(reply.operations)
}
