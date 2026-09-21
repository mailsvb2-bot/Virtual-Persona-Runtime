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
}

#[derive(Serialize)]
pub(super) struct LiveKitSpeakRequest<'a> {
    pub(super) script: LiveKitSpeakScript<'a>,
}

#[derive(Serialize)]
pub(super) struct LiveKitSpeakScript<'a> {
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
    pub(super) input: &'a str,
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
