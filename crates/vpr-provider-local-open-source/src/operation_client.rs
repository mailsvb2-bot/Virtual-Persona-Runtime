//! RT1 operation-scoped HTTP control client; no implicit retry of CREATE.
//! This is the single client for the durable worker operation endpoint.
//! It is not permission to enable public RT1 until an actual sender backend
//! and a revocation-serialized visitor egress gate are verified.
use std::time::Duration;

use base64::Engine;

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use vpr_integration::{CancellationProbe, ProviderError, ProviderErrorKind};

#[path = "operation_client_recovery.rs"]
mod recovery;

#[derive(Clone)]
pub struct RemoteOperationClient {
    client: Client,
    base: reqwest::Url,
    token: String,
    persona_id: Option<String>,
    persona_version: Option<u64>,
}

#[derive(Serialize)]
struct Generation<'a> {
    generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    persona_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    persona_version: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicationReadiness {
    ready: bool,
    persona_video_ready: bool,
    #[serde(default)]
    persona_id: Option<String>,
    #[serde(default)]
    persona_version: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct CreateReply {
    operation_id: String,
    session_id: String,
}

#[derive(Debug, Deserialize)]
pub struct RemoteIceServer {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RemoteOffer {
    pub kind: String,
    pub sdp: String,
    pub ice_servers: Vec<RemoteIceServer>,
}

#[derive(Debug, Deserialize)]
struct OfferReply {
    operation_id: String,
    session_id: String,
    #[serde(rename = "type")]
    kind: String,
    sdp: String,
    #[serde(default)]
    ice_servers: Vec<RemoteIceServer>,
}

#[derive(Debug, Deserialize)]
struct AnswerReply {
    operation_id: String,
    accepted: bool,
}

#[derive(Debug, Deserialize)]
struct PcmReply {
    operation_id: String,
    accepted_bytes: usize,
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
            persona_id: None,
            persona_version: None,
        })
    }

    /// Bind this client to the exact, canonical, server-retained reviewed Persona.
    /// Every readiness and CREATE uses this binding; no visitor-provided
    /// identity may override it.
    ///
    /// # Errors
    /// Denies missing or malformed Persona identities and zero versions.
    pub fn bind_persona(
        mut self,
        persona_id: &str,
        persona_version: u64,
    ) -> Result<Self, ProviderError> {
        // Exactly mirror vpr-domain::PersonaId: Unicode scalar count,
        // nonblank after trimming, at most 256 scalars. Never introduce
        // an ASCII-only alias of canonical owner identity.
        if persona_id.trim().is_empty() || persona_id.chars().count() > 256 || persona_version == 0
        {
            return Err(denied());
        }
        if self.persona_id.is_some() {
            return Err(denied());
        }
        self.persona_id = Some(persona_id.to_owned());
        self.persona_version = Some(persona_version);
        Ok(self)
    }

    /// Authenticated, read-only worker admission preflight. This cannot
    /// allocate media or certify remote STOP, and CREATE always rechecks
    /// durable admission under its own `SQLite` transaction.
    ///
    /// # Errors
    /// Rejects unreachable, unauthorized, busy or malformed worker replies.
    pub fn publication_ready(&self) -> Result<(), &'static str> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|()| "RT1_WORKER_NOT_READY")?
            .extend(["v1", "avatar", "readiness"]);
        let response = self
            .client
            .get(url)
            .bearer_auth(&self.token)
            .send()
            .map_err(|_| "RT1_WORKER_NOT_READY")?;
        let status = response.status();
        let result: PublicationReadiness =
            super::decode_json_response(response, None).map_err(|_| "RT1_WORKER_NOT_READY")?;
        if !result.persona_video_ready {
            return Err("RT1_VIDEO_RENDERER_UNAVAILABLE");
        }
        if let Some(expected) = &self.persona_id {
            if result.persona_id.as_deref() != Some(expected.as_str())
                || result.persona_version != self.persona_version
            {
                return Err("RT1_PERSONA_RENDERER_IDENTITY_MISMATCH");
            }
        }
        if status != reqwest::StatusCode::OK || !result.ready {
            return Err("RT1_WORKER_NOT_READY");
        }
        Ok(())
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
        // Keep the readiness fence inside the single client that owns CREATE:
        // stale visitor tokens and alternate callers cannot allocate media
        // while the real Persona renderer or durable worker admission is down.
        self.publication_ready().map_err(|_| denied())?;
        self.create_after_readiness(operation_id, generation)
    }

    /// Cancellation-aware admission for the existing avatar adapter. A
    /// cancellation before the durable PUT prevents CREATE; once PUT starts,
    /// its late session id is returned so the canonical owner can compensate
    /// it before any offer/answer is exposed.
    ///
    /// # Errors
    /// Rejects cancellation before CREATE, unavailable readiness, or an
    /// ambiguous/non-matching CREATE acknowledgement. Never retries PUT.
    pub fn create_cancellable(
        &self,
        operation_id: &str,
        generation: u64,
        cancellation: &dyn CancellationProbe,
    ) -> Result<String, ProviderError> {
        if generation == 0 {
            return Err(denied());
        }
        if cancellation.is_cancelled() {
            return Err(ProviderError {
                kind: ProviderErrorKind::Cancelled,
                retryable: false,
            });
        }
        self.publication_ready().map_err(|_| denied())?;
        if cancellation.is_cancelled() {
            return Err(ProviderError {
                kind: ProviderErrorKind::Cancelled,
                retryable: false,
            });
        }
        self.create_after_readiness(operation_id, generation)
    }

    fn create_after_readiness(
        &self,
        operation_id: &str,
        generation: u64,
    ) -> Result<String, ProviderError> {
        let response = self
            .client
            .put(self.url(operation_id)?)
            .bearer_auth(&self.token)
            .json(&Generation {
                generation,
                persona_id: self.persona_id.as_deref(),
                persona_version: self.persona_version,
            })
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
            ice_servers: reply.ice_servers,
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
        // A successful HTTP transport is not proof that the worker accepted
        // the SDP. Reject 202/204 and a stale or negative operation receipt.
        if response.status() != reqwest::StatusCode::OK {
            return Err(denied());
        }
        let reply: AnswerReply =
            super::decode_json_response(response, None).map_err(|_| denied())?;
        if reply.operation_id != operation_id || !reply.accepted {
            return Err(denied());
        }
        Ok(())
    }

    /// Queue one bounded PCM16/16kHz/mono utterance fragment on the exact
    /// canonical worker session, under its publication/STOP transaction.
    /// This does not open a second WebRTC sender or authorize a visitor turn.
    ///
    /// # Errors
    /// Denies stale operations, invalid audio, rejected acknowledgements and
    /// cancellations. No automatic retry of ambiguous provider writes.
    pub fn push_pcm(&self, operation_id: &str, pcm: &[u8]) -> Result<(), ProviderError> {
        self.push_pcm_fragment(operation_id, pcm, false)
    }

    /// The final authorized fragment ends the current TTS utterance.
    /// The existing worker may then flush its incomplete 400 ms lip-sync
    /// window; it cannot guess the boundary from transport-idle timing.
    ///
    /// # Errors
    /// Denies invalid or stale operation identities, malformed PCM, worker
    /// rejection, ambiguous transport delivery or a revoked sender.
    pub fn push_pcm_final(&self, operation_id: &str, pcm: &[u8]) -> Result<(), ProviderError> {
        self.push_pcm_fragment(operation_id, pcm, true)
    }

    fn push_pcm_fragment(
        &self,
        operation_id: &str,
        pcm: &[u8],
        final_fragment: bool,
    ) -> Result<(), ProviderError> {
        if pcm.len() < 640 || pcm.len() > 32_000 || pcm.len() % 640 != 0 {
            return Err(denied());
        }
        let mut url = self.url(operation_id)?;
        url.path_segments_mut().map_err(|()| denied())?.push("pcm");
        let response = self
            .client
            .post(url)
            .bearer_auth(&self.token)
            .json(&serde_json::json!({
                "pcm_base64": base64::engine::general_purpose::STANDARD.encode(pcm),
                "sample_rate_hz": 16000,
                "channels": 1,
                "final": final_fragment,
            }))
            .send()
            .map_err(|_| denied())?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(denied());
        }
        let reply: PcmReply = super::decode_json_response(response, None).map_err(|_| denied())?;
        if reply.operation_id != operation_id || reply.accepted_bytes != pcm.len() {
            return Err(denied());
        }
        Ok(())
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

#[cfg(test)]
mod persona_binding_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn probe(body: &'static str, persona_id: &str, version: u64) -> Result<(), &'static str> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut data = [0; 2048];
            let read = stream.read(&mut data).unwrap();
            assert!(
                String::from_utf8_lossy(&data[..read]).starts_with("GET /v1/avatar/readiness ")
            );
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
        });
        let result = RemoteOperationClient::new(&endpoint, "local-token", Duration::from_secs(2))
            .unwrap()
            .bind_persona(persona_id, version)
            .unwrap()
            .publication_ready();
        worker.join().unwrap();
        result
    }

    #[test]
    fn canonical_persona_binding_rejects_stale_and_cross_owner_readiness() {
        let actual = r#"{"ready":true,"persona_video_ready":true,"persona_id":"owner-a","persona_version":3}"#;
        assert_eq!(probe(actual, "owner-a", 3), Ok(()));
        assert_eq!(
            probe(actual, "owner-b", 3),
            Err("RT1_PERSONA_RENDERER_IDENTITY_MISMATCH")
        );
        assert_eq!(
            probe(actual, "owner-a", 2),
            Err("RT1_PERSONA_RENDERER_IDENTITY_MISMATCH")
        );
        assert_eq!(
            probe(r#"{"ready":true,"persona_video_ready":true}"#, "owner-a", 3),
            Err("RT1_PERSONA_RENDERER_IDENTITY_MISMATCH")
        );
    }

    #[test]
    fn malformed_bound_persona_identity_is_rejected_before_worker_io() {
        let client =
            RemoteOperationClient::new("http://127.0.0.1:1", "local-token", Duration::from_secs(2))
                .unwrap();
        assert!(client.clone().bind_persona("", 1).is_err());
        assert!(client.clone().bind_persona(" ", 1).is_err());
        assert!(client.clone().bind_persona(&"Ж".repeat(257), 1).is_err());
        assert!(client.clone().bind_persona("Говорящая персона", 2).is_ok());
        assert!(client.clone().bind_persona("owner-a", 0).is_err());
    }
}
