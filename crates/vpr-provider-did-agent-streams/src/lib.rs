use std::time::Duration;

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use vpr_integration::{
    CancellationProbe, ProviderDescriptor, ProviderError, ProviderErrorKind,
    RealtimeAvatarCapabilities, RealtimeAvatarCapability, RealtimeAvatarClientCommand,
    RealtimeAvatarClientControl, RealtimeAvatarClientEvent, RealtimeAvatarClientRoute,
    RealtimeAvatarPort, RealtimeAvatarSession, RealtimeAvatarTransport, WebRtcIceCandidate,
    WebRtcIceServer, WebRtcSessionDescription,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

mod client_control;

use client_control::DidClientControlRegistry;

pub struct DidAgentStreamsConfig {
    endpoint: String,
    api_key: String,
    agent_id: String,
    fluent: bool,
    timeout: Duration,
}

impl DidAgentStreamsConfig {
    #[must_use]
    pub fn new(
        endpoint: impl Into<String>,
        api_key: impl Into<String>,
        agent_id: impl Into<String>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            api_key: api_key.into(),
            agent_id: agent_id.into(),
            fluent: false,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    #[must_use]
    pub const fn with_fluent(mut self, fluent: bool) -> Self {
        self.fluent = fluent;
        self
    }

    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[must_use]
    pub fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider: "d-id-agents-streams".to_owned(),
            model: "agents-streams".to_owned(),
            representation: Some(self.agent_id.clone()),
        }
    }
}

pub struct DidAgentStreamsAvatar {
    client: Client,
    config: DidAgentStreamsConfig,
    base_url: reqwest::Url,
    client_control: DidClientControlRegistry,
}

impl DidAgentStreamsAvatar {
    /// Builds the D-ID Agents Streams control-plane adapter.
    ///
    /// # Errors
    /// Returns a typed configuration/provider error for invalid endpoints or HTTP client setup.
    pub fn new(config: DidAgentStreamsConfig) -> Result<Self, ProviderError> {
        let base_url = validate_endpoint(&config.endpoint)?;
        if config.agent_id.trim().is_empty() || config.api_key.trim().is_empty() {
            return Err(invalid_response());
        }
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|error| map_transport_error(&error))?;
        Ok(Self {
            client,
            config,
            base_url,
            client_control: DidClientControlRegistry::default(),
        })
    }

    fn agent_url(&self) -> Result<reqwest::Url, ProviderError> {
        let mut url = self.base_url.clone();
        {
            let mut segments = url.path_segments_mut().map_err(|()| invalid_response())?;
            segments.pop_if_empty();
            segments.extend(["agents", self.config.agent_id.as_str()]);
        }
        Ok(url)
    }

    fn streams_url(&self) -> Result<reqwest::Url, ProviderError> {
        let mut url = self.agent_url()?;
        url.path_segments_mut()
            .map_err(|()| invalid_response())?
            .push("streams");
        Ok(url)
    }

    fn v2_sessions_url(&self) -> Result<reqwest::Url, ProviderError> {
        let mut url = self.base_url.clone();
        {
            let mut segments = url.path_segments_mut().map_err(|()| invalid_response())?;
            segments.pop_if_empty();
            segments.extend(["v2", "agents", self.config.agent_id.as_str(), "sessions"]);
        }
        Ok(url)
    }

    fn stream_url(&self, stream_id: &str) -> Result<reqwest::Url, ProviderError> {
        let mut url = self.streams_url()?;
        url.path_segments_mut()
            .map_err(|()| invalid_response())?
            .push(stream_id);
        Ok(url)
    }

    fn stream_subresource_url(
        &self,
        stream_id: &str,
        resource: &str,
    ) -> Result<reqwest::Url, ProviderError> {
        let mut url = self.stream_url(stream_id)?;
        url.path_segments_mut()
            .map_err(|()| invalid_response())?
            .push(resource);
        Ok(url)
    }

    fn validate_session(session: &RealtimeAvatarSession) -> Result<(), ProviderError> {
        if session.provider_resource_id.trim().is_empty()
            || session.provider_session_id.trim().is_empty()
        {
            Err(invalid_response())
        } else {
            Ok(())
        }
    }

    fn presenter_type(&self) -> Result<String, ProviderError> {
        let response = self
            .authorized(self.client.get(self.agent_url()?))
            .send()
            .map_err(|error| map_transport_error(&error))?;
        let response = Self::expect_success(response)?;
        let body: AgentResponse = response.json().map_err(|_| invalid_response())?;
        let presenter_type = body.presenter.kind.trim().to_ascii_lowercase();
        if presenter_type.is_empty() {
            Err(invalid_response())
        } else {
            Ok(presenter_type)
        }
    }

    fn create_webrtc_session(
        &self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarSession, ProviderError> {
        Self::ensure_active(cancellation)?;
        let response = self
            .authorized(self.client.post(self.streams_url()?))
            .json(&CreateStreamRequest {
                fluent: self.config.fluent,
            })
            .send()
            .map_err(|error| map_transport_error(&error))?;
        let response = Self::expect_success(response)?;
        let body: CreateStreamResponse = response.json().map_err(|_| invalid_response())?;
        let client_interrupt = body.fluent && body.interrupt_enabled;
        let session: RealtimeAvatarSession = body.try_into()?;
        if client_interrupt {
            self.client_control.register_interrupt(&session)?;
        }
        Ok(session)
    }

    fn create_livekit_session(
        &self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarSession, ProviderError> {
        Self::ensure_active(cancellation)?;
        let response = self
            .authorized(self.client.post(self.v2_sessions_url()?))
            .send()
            .map_err(|error| map_transport_error(&error))?;
        let response = Self::expect_success(response)?;
        let body: CreateV2SessionResponse = response.json().map_err(|_| invalid_response())?;
        body.try_into()
    }

    fn authorized(&self, request: RequestBuilder) -> RequestBuilder {
        request
            .header(AUTHORIZATION, format!("Basic {}", self.config.api_key))
            .header(CONTENT_TYPE, "application/json")
    }

    fn ensure_active(cancellation: &dyn CancellationProbe) -> Result<(), ProviderError> {
        if cancellation.is_cancelled() {
            Err(cancelled())
        } else {
            Ok(())
        }
    }

    fn expect_success(response: Response) -> Result<Response, ProviderError> {
        if response.status().is_success() {
            Ok(response)
        } else {
            Err(map_status(response.status().as_u16()))
        }
    }
}

impl RealtimeAvatarPort for DidAgentStreamsAvatar {
    fn descriptor(&self) -> ProviderDescriptor {
        self.config.descriptor()
    }

    fn capabilities(&self) -> RealtimeAvatarCapabilities {
        RealtimeAvatarCapabilities::new([
            RealtimeAvatarCapability::TextInput,
            RealtimeAvatarCapability::AudioUrlInput,
        ])
    }

    fn create_session(
        &self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarSession, ProviderError> {
        Self::ensure_active(cancellation)?;
        if self.presenter_type()? == "expressive" {
            self.create_livekit_session(cancellation)
        } else {
            self.create_webrtc_session(cancellation)
        }
    }

    fn submit_answer(
        &self,
        session: &RealtimeAvatarSession,
        answer: &WebRtcSessionDescription,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        Self::ensure_active(cancellation)?;
        Self::validate_session(session)?;
        if !matches!(session.transport, RealtimeAvatarTransport::WebRtc { .. }) {
            return Err(unavailable());
        }
        if answer.kind.trim().is_empty() || answer.sdp.trim().is_empty() {
            return Err(invalid_response());
        }
        let response = self
            .authorized(
                self.client
                    .post(self.stream_subresource_url(&session.provider_resource_id, "sdp")?),
            )
            .json(&SdpRequest {
                session_id: &session.provider_session_id,
                answer: SessionDescriptionRef {
                    kind: &answer.kind,
                    sdp: &answer.sdp,
                },
            })
            .send()
            .map_err(|error| map_transport_error(&error))?;
        Self::expect_success(response).map(|_| ())
    }

    fn submit_ice_candidate(
        &self,
        session: &RealtimeAvatarSession,
        candidate: &WebRtcIceCandidate,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        Self::ensure_active(cancellation)?;
        Self::validate_session(session)?;
        if !matches!(session.transport, RealtimeAvatarTransport::WebRtc { .. }) {
            return Err(unavailable());
        }
        let response = self
            .authorized(
                self.client
                    .post(self.stream_subresource_url(&session.provider_resource_id, "ice")?),
            )
            .json(&IceRequest {
                session_id: &session.provider_session_id,
                candidate: candidate.candidate.as_deref(),
                sdp_mid: candidate.sdp_mid.as_deref(),
                sdp_mline_index: candidate.sdp_mline_index,
            })
            .send()
            .map_err(|error| map_transport_error(&error))?;
        Self::expect_success(response).map(|_| ())
    }

    fn speak_text(
        &self,
        session: &RealtimeAvatarSession,
        text: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        Self::ensure_active(cancellation)?;
        Self::validate_session(session)?;
        if text.trim().is_empty() {
            return Err(invalid_response());
        }
        if !matches!(session.transport, RealtimeAvatarTransport::WebRtc { .. }) {
            return Err(unavailable());
        }
        let response = self
            .authorized(
                self.client
                    .post(self.stream_url(&session.provider_resource_id)?),
            )
            .json(&SpeakRequest {
                session_id: &session.provider_session_id,
                script: SpeakScript::Text { input: text },
            })
            .send()
            .map_err(|error| map_transport_error(&error))?;
        Self::expect_success(response).map(|_| ())
    }

    fn speak_audio_url(
        &self,
        session: &RealtimeAvatarSession,
        audio_url: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        Self::ensure_active(cancellation)?;
        Self::validate_session(session)?;
        validate_audio_url(audio_url)?;
        if !matches!(session.transport, RealtimeAvatarTransport::WebRtc { .. }) {
            return Err(unavailable());
        }
        let response = self
            .authorized(
                self.client
                    .post(self.stream_url(&session.provider_resource_id)?),
            )
            .json(&SpeakRequest {
                session_id: &session.provider_session_id,
                script: SpeakScript::Audio { audio_url },
            })
            .send()
            .map_err(|error| map_transport_error(&error))?;
        Self::expect_success(response).map(|_| ())
    }

    fn client_control(
        &self,
        session: &RealtimeAvatarSession,
    ) -> Option<RealtimeAvatarClientControl> {
        Self::validate_session(session).ok()?;
        match session.transport {
            RealtimeAvatarTransport::LiveKit { .. } => Some(RealtimeAvatarClientControl {
                event_route: None,
                interrupt: true,
                interrupt_requires_playback_id: false,
                text_input: true,
            }),
            RealtimeAvatarTransport::WebRtc { .. } => self.client_control.control(session),
        }
    }

    fn parse_client_event(
        &self,
        session: &RealtimeAvatarSession,
        message: &str,
    ) -> Result<Option<RealtimeAvatarClientEvent>, ProviderError> {
        Self::validate_session(session)?;
        match session.transport {
            RealtimeAvatarTransport::LiveKit { .. } => parse_livekit_event(message),
            RealtimeAvatarTransport::WebRtc { .. } => self.client_control.parse_event(session, message),
        }
    }

    fn prepare_client_text(
        &self,
        session: &RealtimeAvatarSession,
        text: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarClientCommand, ProviderError> {
        Self::ensure_active(cancellation)?;
        Self::validate_session(session)?;
        let text = text.trim();
        if text.is_empty() {
            return Err(invalid_response());
        }
        if !matches!(session.transport, RealtimeAvatarTransport::LiveKit { .. }) {
            return Err(unavailable());
        }
        let payload = serde_json::to_string(&LiveKitSpeakRequest {
            script: LiveKitSpeakScript {
                kind: "text",
                input: text,
            },
        })
        .map_err(|_| invalid_response())?;
        Ok(RealtimeAvatarClientCommand {
            route: RealtimeAvatarClientRoute::LiveKitTextTopic {
                topic: "did.speak".to_owned(),
            },
            payload,
        })
    }

    fn prepare_client_interrupt(
        &self,
        session: &RealtimeAvatarSession,
        playback_id: Option<&str>,
        cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarClientCommand, ProviderError> {
        Self::ensure_active(cancellation)?;
        Self::validate_session(session)?;
        match session.transport {
            RealtimeAvatarTransport::LiveKit { .. } => Ok(RealtimeAvatarClientCommand {
                route: RealtimeAvatarClientRoute::LiveKitTextTopic {
                    topic: "did.interrupt".to_owned(),
                },
                payload: "{}".to_owned(),
            }),
            RealtimeAvatarTransport::WebRtc { .. } => self
                .client_control
                .prepare_interrupt(session, playback_id.ok_or_else(invalid_response)?),
        }
    }

    fn close_session(&self, session: &RealtimeAvatarSession) -> Result<(), ProviderError> {
        Self::validate_session(session)?;
        match session.transport {
            RealtimeAvatarTransport::LiveKit { .. } => {
                // D-ID V2 LiveKit sessions have no explicit delete endpoint. The browser disconnects
                // from the room and D-ID closes the unused session after its inactivity timeout.
                Ok(())
            }
            RealtimeAvatarTransport::WebRtc { .. } => {
                let response = self
                    .authorized(
                        self.client
                            .delete(self.stream_url(&session.provider_resource_id)?),
                    )
                    .json(&CloseRequest {
                        session_id: &session.provider_session_id,
                    })
                    .send()
                    .map_err(|error| map_transport_error(&error))?;
                Self::expect_success(response)?;
                self.client_control.forget(session)
            }
        }
    }
}

#[derive(Deserialize)]
struct AgentResponse {
    presenter: AgentPresenter,
}

#[derive(Deserialize)]
struct AgentPresenter {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct CreateV2SessionResponse {
    id: String,
    session_url: String,
    session_token: String,
}

#[derive(Serialize)]
struct LiveKitSpeakRequest<'a> {
    script: LiveKitSpeakScript<'a>,
}

#[derive(Serialize)]
struct LiveKitSpeakScript<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    input: &'a str,
}

#[derive(Deserialize)]
struct LiveKitEvent {
    subject: Option<String>,
}

#[derive(Serialize)]
struct CreateStreamRequest {
    fluent: bool,
}

#[derive(Deserialize)]
struct CreateStreamResponse {
    id: String,
    session_id: String,
    offer: SessionDescriptionOwned,
    #[serde(default)]
    ice_servers: Vec<IceServerResponse>,
    #[serde(default)]
    fluent: bool,
    #[serde(default)]
    interrupt_enabled: bool,
}

#[derive(Deserialize)]
struct SessionDescriptionOwned {
    #[serde(rename = "type")]
    kind: String,
    sdp: String,
}

#[derive(Deserialize)]
struct IceServerResponse {
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

impl TryFrom<CreateStreamResponse> for RealtimeAvatarSession {
    type Error = ProviderError;

    fn try_from(value: CreateStreamResponse) -> Result<Self, Self::Error> {
        if value.id.trim().is_empty()
            || value.session_id.trim().is_empty()
            || value.offer.kind.trim().is_empty()
            || value.offer.sdp.trim().is_empty()
        {
            return Err(invalid_response());
        }
        Ok(Self {
            provider_resource_id: value.id,
            provider_session_id: value.session_id,
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

impl TryFrom<CreateV2SessionResponse> for RealtimeAvatarSession {
    type Error = ProviderError;

    fn try_from(value: CreateV2SessionResponse) -> Result<Self, Self::Error> {
        let session_id = value.id.trim();
        if session_id.is_empty()
            || value.session_token.trim().is_empty()
            || !valid_livekit_url(&value.session_url)
        {
            return Err(invalid_response());
        }
        Ok(Self {
            provider_resource_id: session_id.to_owned(),
            provider_session_id: session_id.to_owned(),
            transport: RealtimeAvatarTransport::LiveKit {
                server_url: value.session_url,
                token: value.session_token,
            },
        })
    }
}

#[derive(Serialize)]
struct SdpRequest<'a> {
    session_id: &'a str,
    answer: SessionDescriptionRef<'a>,
}

#[derive(Serialize)]
struct SessionDescriptionRef<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    sdp: &'a str,
}

#[derive(Serialize)]
struct IceRequest<'a> {
    session_id: &'a str,
    candidate: Option<&'a str>,
    #[serde(rename = "sdpMid")]
    sdp_mid: Option<&'a str>,
    #[serde(rename = "sdpMLineIndex")]
    sdp_mline_index: Option<u16>,
}

#[derive(Serialize)]
struct SpeakRequest<'a> {
    session_id: &'a str,
    script: SpeakScript<'a>,
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum SpeakScript<'a> {
    #[serde(rename = "text")]
    Text { input: &'a str },
    #[serde(rename = "audio")]
    Audio { audio_url: &'a str },
}

#[derive(Serialize)]
struct CloseRequest<'a> {
    session_id: &'a str,
}

fn parse_livekit_event(message: &str) -> Result<Option<RealtimeAvatarClientEvent>, ProviderError> {
    if message.len() > 64 * 1024 {
        return Err(invalid_response());
    }
    let event: LiveKitEvent = serde_json::from_str(message).map_err(|_| invalid_response())?;
    match event.subject.as_deref() {
        Some("stream-video/done") => Ok(Some(RealtimeAvatarClientEvent::PlaybackDone)),
        _ => Ok(None),
    }
}

fn valid_livekit_url(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "wss"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
    })
}

fn validate_audio_url(audio_url: &str) -> Result<(), ProviderError> {
    let parsed = reqwest::Url::parse(audio_url).map_err(|_| invalid_response())?;
    let has_host = parsed
        .host_str()
        .is_some_and(|host| !host.trim().is_empty());
    let has_userinfo = !parsed.username().is_empty() || parsed.password().is_some();
    if parsed.scheme() == "https" && has_host && !has_userinfo {
        Ok(())
    } else {
        Err(policy_denied())
    }
}

fn validate_endpoint(endpoint: &str) -> Result<reqwest::Url, ProviderError> {
    let parsed = reqwest::Url::parse(endpoint).map_err(|_| invalid_response())?;
    let loopback = parsed.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    let secure_scheme = parsed.scheme() == "https" || (parsed.scheme() == "http" && loopback);
    let clean_authority = parsed.username().is_empty() && parsed.password().is_none();
    if secure_scheme
        && clean_authority
        && parsed.host_str().is_some()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
    {
        Ok(parsed)
    } else {
        Err(policy_denied())
    }
}

fn map_status(status: u16) -> ProviderError {
    match status {
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
    }
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

const fn cancelled() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::Cancelled,
        retryable: false,
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

#[cfg(test)]
mod tests;
