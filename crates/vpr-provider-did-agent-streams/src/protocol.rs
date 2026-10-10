use serde::{Deserialize, Serialize};
use vpr_integration::{
    ProviderError, RealtimeAvatarClientEvent, RealtimeAvatarSession, RealtimeAvatarTransport,
    WebRtcIceServer, WebRtcSessionDescription,
};

use super::invalid_response;

#[derive(Deserialize)]
pub(super) struct AgentResponse {
    pub(super) presenter: AgentPresenter,
}

#[derive(Deserialize)]
pub(super) struct AgentPresenter {
    #[serde(rename = "type")]
    pub(super) kind: String,
}

#[derive(Deserialize)]
pub(super) struct CreateV2SessionResponse {
    pub(super) id: String,
    pub(super) session_url: String,
    pub(super) session_token: String,
    /// Only in Echo sessions. Must stay on the server; the browser receives
    /// the ordinary `session_token` and is not authorized as speech sender.
    #[serde(default)]
    pub(super) echo_token: Option<String>,
}

#[derive(Deserialize)]
struct LiveKitEvent {
    subject: Option<String>,
}

#[derive(Serialize)]
pub(super) struct CreateStreamRequest {
    pub(super) fluent: bool,
}

#[derive(Deserialize)]
pub(super) struct CreateStreamResponse {
    pub(super) id: String,
    pub(super) session_id: String,
    pub(super) offer: SessionDescriptionOwned,
    #[serde(default)]
    pub(super) ice_servers: Vec<IceServerResponse>,
    #[serde(default)]
    pub(super) fluent: bool,
    #[serde(default)]
    pub(super) interrupt_enabled: bool,
}

#[derive(Deserialize)]
pub(super) struct SessionDescriptionOwned {
    #[serde(rename = "type")]
    kind: String,
    sdp: String,
}

#[derive(Deserialize)]
pub(super) struct IceServerResponse {
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
        if session_id.is_empty() || value.session_token.trim().is_empty() {
            return Err(invalid_response());
        }
        let server_url = livekit_connection_url(&value.session_url)?;
        Ok(Self {
            provider_resource_id: session_id.to_owned(),
            provider_session_id: session_id.to_owned(),
            transport: RealtimeAvatarTransport::LiveKit {
                server_url,
                token: value.session_token,
            },
        })
    }
}

#[derive(Serialize)]
pub(super) struct SdpRequest<'a> {
    pub(super) session_id: &'a str,
    pub(super) answer: SessionDescriptionRef<'a>,
}

#[derive(Serialize)]
pub(super) struct SessionDescriptionRef<'a> {
    #[serde(rename = "type")]
    pub(super) kind: &'a str,
    pub(super) sdp: &'a str,
}

#[derive(Serialize)]
pub(super) struct IceRequest<'a> {
    pub(super) session_id: &'a str,
    pub(super) candidate: Option<&'a str>,
    #[serde(rename = "sdpMid")]
    pub(super) sdp_mid: Option<&'a str>,
    #[serde(rename = "sdpMLineIndex")]
    pub(super) sdp_mline_index: Option<u16>,
}

#[derive(Serialize)]
pub(super) struct SpeakRequest<'a> {
    pub(super) session_id: &'a str,
    pub(super) script: SpeakScript<'a>,
}

#[derive(Serialize)]
#[serde(tag = "type")]
pub(super) enum SpeakScript<'a> {
    #[serde(rename = "text")]
    Text { input: &'a str },
    #[serde(rename = "audio")]
    Audio { audio_url: &'a str },
}

#[derive(Serialize)]
pub(super) struct CloseRequest<'a> {
    pub(super) session_id: &'a str,
}

pub(super) fn parse_livekit_event(
    message: &str,
) -> Result<Option<RealtimeAvatarClientEvent>, ProviderError> {
    if message.len() > 64 * 1024 {
        return Err(invalid_response());
    }
    let event: LiveKitEvent = serde_json::from_str(message).map_err(|_| invalid_response())?;
    match event.subject.as_deref() {
        Some("stream-video/started") => Ok(Some(RealtimeAvatarClientEvent::VideoGenerationStarted)),
        Some("stream-video/done") => Ok(Some(RealtimeAvatarClientEvent::VideoGenerationDone)),
        Some("stream-video/error") => Ok(Some(RealtimeAvatarClientEvent::VideoGenerationFailed)),
        Some(
            "chat/answer"
            | "chat/partial"
            | "chat/audio-transcribed"
            | "tool-call/started"
            | "tool-call/done"
            | "tool-call/error",
        ) => Ok(Some(RealtimeAvatarClientEvent::Informational)),
        Some(subject) if subject.starts_with("chat/") => {
            Ok(Some(RealtimeAvatarClientEvent::UnknownChatEvent))
        }
        Some(subject) if subject.starts_with("stream-video/") => {
            Ok(Some(RealtimeAvatarClientEvent::UnknownVideoEvent))
        }
        Some(subject) if subject.starts_with("tool-call/") => {
            Ok(Some(RealtimeAvatarClientEvent::UnknownToolEvent))
        }
        // No raw subject or content crosses the canonical diagnostic boundary.
        _ => Ok(Some(RealtimeAvatarClientEvent::UnknownOtherEvent)),
    }
}

fn livekit_connection_url(value: &str) -> Result<String, ProviderError> {
    let mut url = reqwest::Url::parse(value).map_err(|_| invalid_response())?;
    if url.scheme() != "wss"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
    {
        return Err(invalid_response());
    }
    // D-ID Echo returns a session_url with /room/<identity>. LiveKit's
    // Room.connect needs the websocket SERVER origin, not that room path.
    // The room identity is already carried by the signed session token.
    if url.path().starts_with("/room/") {
        url.set_path("/");
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}
