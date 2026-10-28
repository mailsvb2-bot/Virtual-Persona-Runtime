//! RT1 operation-scoped HTTP control client; no implicit retry of CREATE.
//! This is the single client for the durable worker operation endpoint.
//! It is not permission to enable public RT1 until an actual sender backend
//! and a revocation-serialized visitor egress gate are verified.
use std::time::Duration;

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use vpr_integration::{ProviderError, ProviderErrorKind};

#[derive(Clone)]
pub struct RemoteOperationClient {
    client: Client,
    base: reqwest::Url,
    token: String,
}

#[derive(Serialize)]
struct Generation {
    generation: u64,
}

#[derive(Debug, Deserialize)]
struct CreateReply {
    operation_id: String,
    session_id: String,
}

#[derive(Debug, Deserialize)]
pub struct RemoteOffer {
    pub kind: String,
    pub sdp: String,
}

#[derive(Debug, Deserialize)]
struct OfferReply {
    operation_id: String,
    session_id: String,
    #[serde(rename = "type")]
    kind: String,
    sdp: String,
}

#[derive(Debug, Deserialize)]
struct CancelReply {
    operation_id: String,
    session_id: String,
    generation: u64,
    audio_stopped: bool,
    video_stopped: bool,
}

#[derive(Debug, Deserialize)]
pub struct OperationStatus {
    pub operation_id: String,
    pub generation: u64,
    pub phase: String,
    pub session_id: Option<String>,
    pub stop_confirmed: bool,
}

fn denied() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::Unavailable,
        retryable: false,
    }
}

impl RemoteOperationClient {
    /// # Errors
    /// Refuses insecure remote endpoints, credentials in URLs, and missing token.
    pub fn new(endpoint: &str, token: &str, timeout: Duration) -> Result<Self, ProviderError> {
        let base = super::validate_endpoint(endpoint)?;
        if token.trim().is_empty() || timeout.is_zero() {
            return Err(denied());
        }
        let client = super::build_provider_http_client(timeout).map_err(|_| denied())?;
        Ok(Self {
            client,
            base,
            token: token.to_owned(),
        })
    }

    fn url(&self, operation_id: &str) -> Result<reqwest::Url, ProviderError> {
        if operation_id.is_empty()
            || operation_id.len() > 128
            || !operation_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(denied());
        }
        let mut url = self.base.clone();
        url.path_segments_mut().map_err(|()| denied())?.extend([
            "v1",
            "avatar",
            "operations",
            operation_id,
        ]);
        Ok(url)
    }

    /// # Errors
    /// A missing response or unexpected acknowledgement is never retryable.
    pub fn create(&self, operation_id: &str, generation: u64) -> Result<String, ProviderError> {
        if generation == 0 {
            return Err(denied());
        }
        let response = self
            .client
            .put(self.url(operation_id)?)
            .bearer_auth(&self.token)
            .json(&Generation { generation })
            .send()
            .map_err(|_| denied())?;
        if !response.status().is_success() {
            return Err(denied());
        }
        let reply: CreateReply =
            super::decode_json_response(response, None).map_err(|_| denied())?;
        if reply.operation_id != operation_id || reply.session_id.is_empty() {
            return Err(denied());
        }
        Ok(reply.session_id)
    }

    /// Fetch an offer for a specific, previously allocated operation.
    /// # Errors
    /// Denies mismatched sessions or malformed SDP responses.
    pub fn offer(
        &self,
        operation_id: &str,
        session_id: &str,
    ) -> Result<RemoteOffer, ProviderError> {
        let mut url = self.url(operation_id)?;
        url.path_segments_mut()
            .map_err(|()| denied())?
            .push("offer");
        let response = self
            .client
            .get(url)
            .bearer_auth(&self.token)
            .send()
            .map_err(|_| denied())?;
        if !response.status().is_success() {
            return Err(denied());
        }
        let reply: OfferReply =
            super::decode_json_response(response, None).map_err(|_| denied())?;
        if reply.operation_id != operation_id
            || reply.session_id != session_id
            || reply.kind != "offer"
            || reply.sdp.trim().is_empty()
        {
            return Err(denied());
        }
        Ok(RemoteOffer {
            kind: reply.kind,
            sdp: reply.sdp,
        })
    }

    /// Submit the visitor's answer to the same remote operation.
    /// # Errors
    /// Denies empty SDP or failure to authenticate the active session.
    pub fn answer(&self, operation_id: &str, sdp: &str) -> Result<(), ProviderError> {
        if sdp.trim().is_empty() {
            return Err(denied());
        }
        let mut url = self.url(operation_id)?;
        url.path_segments_mut()
            .map_err(|()| denied())?
            .push("answer");
        let response = self
            .client
            .post(url)
            .bearer_auth(&self.token)
            .json(&serde_json::json!({"type":"answer", "sdp":sdp}))
            .send()
            .map_err(|_| denied())?;
        if !response.status().is_success() {
            return Err(denied());
        }
        Ok(())
    }

    /// # Errors
    /// 202 means pending, never confirmed; requires both positive media receipts.
    pub fn cancel_confirmed(
        &self,
        operation_id: &str,
        generation: u64,
    ) -> Result<String, ProviderError> {
        if generation == 0 {
            return Err(denied());
        }
        let response = self
            .client
            .delete(self.url(operation_id)?)
            .bearer_auth(&self.token)
            .json(&Generation { generation })
            .send()
            .map_err(|_| denied())?;
        if !response.status().is_success() || response.status().as_u16() == 202 {
            return Err(denied());
        }
        let reply: CancelReply =
            super::decode_json_response(response, None).map_err(|_| denied())?;
        if reply.operation_id != operation_id
            || reply.generation != generation
            || reply.session_id.is_empty()
            || !reply.audio_stopped
            || !reply.video_stopped
        {
            return Err(denied());
        }
        Ok(reply.session_id)
    }

    /// # Errors
    /// Status is diagnostic only; never substitutes for a signed-off stop call.
    pub fn status(&self, operation_id: &str) -> Result<OperationStatus, ProviderError> {
        let response = self
            .client
            .get(self.url(operation_id)?)
            .bearer_auth(&self.token)
            .send()
            .map_err(|_| denied())?;
        if !response.status().is_success() {
            return Err(denied());
        }
        let state: OperationStatus =
            super::decode_json_response(response, None).map_err(|_| denied())?;
        if state.operation_id != operation_id
            || state.generation == 0
            || !matches!(
                state.phase.as_str(),
                "creating" | "active" | "stopping" | "stopped" | "uncertain"
            )
            || (state.stop_confirmed && (state.phase != "stopped" || state.session_id.is_none()))
        {
            return Err(denied());
        }
        Ok(state)
    }
}
