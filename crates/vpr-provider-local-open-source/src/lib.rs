use std::time::Duration;

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use vpr_integration::{
    CancellationProbe, MAX_PROVIDER_JSON_BODY_BYTES, ProviderDescriptor, ProviderError,
    ProviderErrorKind, RealtimeAvatarCapabilities, RealtimeAvatarCapability, RealtimeAvatarPort,
    RealtimeAvatarSession, RealtimeAvatarTransport, WebRtcIceCandidate, WebRtcIceServer,
    WebRtcSessionDescription, build_provider_http_client, read_bounded_provider_body,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

pub struct LocalOpenSourceAvatarConfig {
    endpoint: String,
    api_token: String,
    timeout: Duration,
}

impl LocalOpenSourceAvatarConfig {
    #[must_use]
    pub fn new(endpoint: impl Into<String>, api_token: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            api_token: api_token.into(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

pub struct LocalOpenSourceAvatar {
    client: Client,
    base_url: reqwest::Url,
    api_token: String,
}

impl LocalOpenSourceAvatar {
    /// Builds the self-hosted realtime avatar adapter.
    ///
    /// The worker contract is intentionally provider-neutral: the remote GPU worker may run
    /// `MuseTalk`, `LivePortrait`, or another local renderer behind the same WebRTC API.
    ///
    /// # Errors
    /// Returns a typed provider error for invalid endpoints, missing authentication, or HTTP setup.
    pub fn new(config: LocalOpenSourceAvatarConfig) -> Result<Self, ProviderError> {
        let base_url = validate_endpoint(&config.endpoint)?;
        if config.api_token.trim().is_empty() {
            return Err(invalid_response());
        }
        let client = build_provider_http_client(config.timeout)
            .map_err(|error| map_transport_error(&error))?;
        Ok(Self {
            client,
            base_url,
            api_token: config.api_token,
        })
    }

    /// Checks authenticated worker reachability without creating a GPU rendering session.
    ///
    /// # Errors
    /// Returns a typed provider error when the worker is unreachable or rejects authentication.
    pub fn probe_health(&self) -> Result<(), ProviderError> {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|()| invalid_response())?
            .extend(["v1", "health"]);
        let response = self
            .authorized(self.client.get(url))
            .send()
            .map_err(|error| map_transport_error(&error))?;
        expect_success(response).map(|_| ())
    }

    fn sessions_url(&self) -> Result<reqwest::Url, ProviderError> {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|()| invalid_response())?
            .extend(["v1", "avatar", "sessions"]);
        Ok(url)
    }

    fn session_url(&self, session_id: &str) -> Result<reqwest::Url, ProviderError> {
        let mut url = self.sessions_url()?;
        url.path_segments_mut()
            .map_err(|()| invalid_response())?
            .push(session_id);
        Ok(url)
    }

    fn subresource_url(
        &self,
        session_id: &str,
        resource: &str,
    ) -> Result<reqwest::Url, ProviderError> {
        let mut url = self.session_url(session_id)?;
        url.path_segments_mut()
            .map_err(|()| invalid_response())?
            .push(resource);
        Ok(url)
    }

    fn authorized(&self, request: RequestBuilder) -> RequestBuilder {
        request
            .header(AUTHORIZATION, format!("Bearer {}", self.api_token))
            .header(CONTENT_TYPE, "application/json")
    }

    fn validate_session(session: &RealtimeAvatarSession) -> Result<&str, ProviderError> {
        let id = session.provider_session_id.trim();
        if id.is_empty() || session.provider_resource_id != session.provider_session_id {
            return Err(invalid_response());
        }
        if !matches!(session.transport, RealtimeAvatarTransport::WebRtc { .. }) {
            return Err(invalid_response());
        }
        Ok(id)
    }

    fn ensure_active(cancellation: &dyn CancellationProbe) -> Result<(), ProviderError> {
        if cancellation.is_cancelled() {
            Err(cancelled())
        } else {
            Ok(())
        }
    }
}

fn decode_json_response<T: DeserializeOwned>(
    response: Response,
    cancellation: Option<&dyn CancellationProbe>,
) -> Result<T, ProviderError> {
    let body =
        read_bounded_provider_body(response, MAX_PROVIDER_JSON_BODY_BYTES, cancellation)?;
    serde_json::from_slice(&body).map_err(|_| invalid_response())
}

impl RealtimeAvatarPort for LocalOpenSourceAvatar {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider: "local-open-source".to_owned(),
            model: "realtime-worker-v1".to_owned(),
            representation: None,
        }
    }

    fn capabilities(&self) -> RealtimeAvatarCapabilities {
        RealtimeAvatarCapabilities::new([
            RealtimeAvatarCapability::TextInput,
            RealtimeAvatarCapability::Interrupt,
        ])
    }

    fn create_session(
        &self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarSession, ProviderError> {
        Self::ensure_active(cancellation)?;
        let response = self
            .authorized(self.client.post(self.sessions_url()?))
            .json(&CreateSessionRequest {})
            .send()
            .map_err(|error| map_transport_error(&error))?;
        let response = expect_success(response)?;
        let body: CreateSessionResponse = decode_json_response(response, Some(cancellation))?;
        body.try_into()
    }

    fn submit_answer(
        &self,
        session: &RealtimeAvatarSession,
        answer: &WebRtcSessionDescription,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        Self::ensure_active(cancellation)?;
        let id = Self::validate_session(session)?;
        if answer.kind != "answer" || answer.sdp.trim().is_empty() {
            return Err(invalid_response());
        }
        let response = self
            .authorized(self.client.post(self.subresource_url(id, "answer")?))
            .json(&AnswerRequest {
                kind: &answer.kind,
                sdp: &answer.sdp,
            })
            .send()
            .map_err(|error| map_transport_error(&error))?;
        expect_success(response).map(|_| ())
    }

    fn submit_ice_candidate(
        &self,
        session: &RealtimeAvatarSession,
        candidate: &WebRtcIceCandidate,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        Self::ensure_active(cancellation)?;
        let id = Self::validate_session(session)?;
        let response = self
            .authorized(self.client.post(self.subresource_url(id, "ice")?))
            .json(&IceRequest {
                candidate: candidate.candidate.as_deref(),
                sdp_mid: candidate.sdp_mid.as_deref(),
                sdp_mline_index: candidate.sdp_mline_index,
            })
            .send()
            .map_err(|error| map_transport_error(&error))?;
        expect_success(response).map(|_| ())
    }

    fn speak_text(
        &self,
        session: &RealtimeAvatarSession,
        text: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        Self::ensure_active(cancellation)?;
        let id = Self::validate_session(session)?;
        let text = text.trim();
        if text.is_empty() {
            return Err(invalid_response());
        }
        let response = self
            .authorized(self.client.post(self.subresource_url(id, "speak")?))
            .json(&SpeakRequest { text })
            .send()
            .map_err(|error| map_transport_error(&error))?;
        expect_success(response).map(|_| ())
    }

    fn speak_audio_url(
        &self,
        _session: &RealtimeAvatarSession,
        _audio_url: &str,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        Err(unavailable())
    }

    fn interrupt(
        &self,
        session: &RealtimeAvatarSession,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        Self::ensure_active(cancellation)?;
        let id = Self::validate_session(session)?;
        let response = self
            .authorized(self.client.post(self.subresource_url(id, "interrupt")?))
            .json(&InterruptRequest {})
            .send()
            .map_err(|error| map_transport_error(&error))?;
        expect_success(response).map(|_| ())
    }

    fn close_session(&self, session: &RealtimeAvatarSession) -> Result<(), ProviderError> {
        let id = Self::validate_session(session)?;
        let response = self
            .authorized(self.client.delete(self.session_url(id)?))
            .send()
            .map_err(|error| map_transport_error(&error))?;
        expect_success(response).map(|_| ())
    }
}

#[derive(Serialize)]
struct CreateSessionRequest {}

#[derive(Deserialize)]
struct CreateSessionResponse {
    id: String,
    offer: SessionDescription,
    #[serde(default)]
    ice_servers: Vec<IceServer>,
}

#[derive(Deserialize)]
struct SessionDescription {
    #[serde(rename = "type")]
    kind: String,
    sdp: String,
}

#[derive(Deserialize)]
struct IceServer {
    #[serde(default)]
    urls: OneOrMany,
    username: Option<String>,
    credential: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
    #[default]
    Missing,
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::One(value) => vec![value],
            Self::Many(values) => values,
            Self::Missing => Vec::new(),
        }
    }
}

impl TryFrom<CreateSessionResponse> for RealtimeAvatarSession {
    type Error = ProviderError;

    fn try_from(value: CreateSessionResponse) -> Result<Self, Self::Error> {
        let id = value.id.trim();
        if id.is_empty() || value.offer.kind != "offer" || value.offer.sdp.trim().is_empty() {
            return Err(invalid_response());
        }
        Ok(Self {
            provider_resource_id: id.to_owned(),
            provider_session_id: id.to_owned(),
            transport: RealtimeAvatarTransport::WebRtc {
                offer: WebRtcSessionDescription {
                    kind: value.offer.kind,
                    sdp: value.offer.sdp,
                },
                ice_servers: value
                    .ice_servers
                    .into_iter()
                    .map(|server| WebRtcIceServer {
                        urls: server.urls.into_vec(),
                        username: server.username,
                        credential: server.credential,
                    })
                    .collect(),
            },
        })
    }
}

#[derive(Serialize)]
struct AnswerRequest<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    sdp: &'a str,
}

#[derive(Serialize)]
struct IceRequest<'a> {
    candidate: Option<&'a str>,
    #[serde(rename = "sdpMid")]
    sdp_mid: Option<&'a str>,
    #[serde(rename = "sdpMLineIndex")]
    sdp_mline_index: Option<u16>,
}

#[derive(Serialize)]
struct SpeakRequest<'a> {
    text: &'a str,
}

#[derive(Serialize)]
struct InterruptRequest {}

fn validate_endpoint(endpoint: &str) -> Result<reqwest::Url, ProviderError> {
    let parsed = reqwest::Url::parse(endpoint).map_err(|_| invalid_response())?;
    let loopback = parsed.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    let secure = parsed.scheme() == "https" || (parsed.scheme() == "http" && loopback);
    if secure
        && parsed.host_str().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
    {
        Ok(parsed)
    } else {
        Err(policy_denied())
    }
}

fn expect_success(response: Response) -> Result<Response, ProviderError> {
    if response.status().is_success() {
        return Ok(response);
    }
    Err(match response.status().as_u16() {
        401 | 403 => policy_denied(),
        408 => ProviderError {
            kind: ProviderErrorKind::Timeout,
            retryable: true,
        },
        429 => ProviderError {
            kind: ProviderErrorKind::RateLimited,
            retryable: true,
        },
        500..=599 => ProviderError {
            kind: ProviderErrorKind::Unavailable,
            retryable: true,
        },
        _ => invalid_response(),
    })
}

fn map_transport_error(error: &reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError {
            kind: ProviderErrorKind::Timeout,
            retryable: true,
        }
    } else {
        ProviderError {
            kind: ProviderErrorKind::Unavailable,
            retryable: true,
        }
    }
}

const fn invalid_response() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::InvalidResponse,
        retryable: false,
    }
}

const fn unavailable() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::Unavailable,
        retryable: false,
    }
}

const fn policy_denied() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::PolicyDenied,
        retryable: false,
    }
}

const fn cancelled() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::Cancelled,
        retryable: false,
    }
}

#[cfg(test)]
mod tests;
