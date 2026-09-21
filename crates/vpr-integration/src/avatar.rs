use std::fmt::{Debug, Formatter, Result as FmtResult};

use crate::{CancellationProbe, ProviderDescriptor, ProviderError, ProviderErrorKind};

#[derive(Clone, PartialEq, Eq)]
pub struct WebRtcSessionDescription {
    pub kind: String,
    pub sdp: String,
}

impl Debug for WebRtcSessionDescription {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter
            .debug_struct("WebRtcSessionDescription")
            .field("kind", &self.kind)
            .field("sdp_bytes", &self.sdp.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct WebRtcIceServer {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

impl Debug for WebRtcIceServer {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter
            .debug_struct("WebRtcIceServer")
            .field("url_count", &self.urls.len())
            .field("username_present", &self.username.is_some())
            .field("credential_present", &self.credential.is_some())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct WebRtcIceCandidate {
    pub candidate: Option<String>,
    pub sdp_mid: Option<String>,
    pub sdp_mline_index: Option<u16>,
}

impl Debug for WebRtcIceCandidate {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter
            .debug_struct("WebRtcIceCandidate")
            .field(
                "candidate_bytes",
                &self.candidate.as_ref().map_or(0, String::len),
            )
            .field("sdp_mid_present", &self.sdp_mid.is_some())
            .field("sdp_mline_index", &self.sdp_mline_index)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum RealtimeAvatarTransport {
    WebRtc {
        offer: WebRtcSessionDescription,
        ice_servers: Vec<WebRtcIceServer>,
    },
    LiveKit {
        server_url: String,
        token: String,
    },
}

impl Debug for RealtimeAvatarTransport {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::WebRtc { offer, ice_servers } => formatter
                .debug_struct("WebRtc")
                .field("offer", offer)
                .field("ice_server_count", &ice_servers.len())
                .finish(),
            Self::LiveKit { server_url, token } => formatter
                .debug_struct("LiveKit")
                .field("server_url_bytes", &server_url.len())
                .field("token_bytes", &token.len())
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RealtimeAvatarSession {
    pub provider_resource_id: String,
    pub provider_session_id: String,
    pub transport: RealtimeAvatarTransport,
}

impl Debug for RealtimeAvatarSession {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter
            .debug_struct("RealtimeAvatarSession")
            .field("resource_id_bytes", &self.provider_resource_id.len())
            .field("session_id_bytes", &self.provider_session_id.len())
            .field("transport", &self.transport)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealtimeAvatarCapability {
    TextInput,
    AudioUrlInput,
    Interrupt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealtimeAvatarClientControl {
    pub event_route: Option<RealtimeAvatarClientRoute>,
    pub interrupt: bool,
    pub interrupt_requires_playback_id: bool,
    pub text_input: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealtimeAvatarClientEvent {
    PlaybackStarted { playback_id: String },
    PlaybackDone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealtimeAvatarClientRoute {
    WebRtcDataChannel { label: String },
    LiveKitTextTopic { topic: String },
}

#[derive(Clone, PartialEq, Eq)]
pub struct RealtimeAvatarClientCommand {
    pub route: RealtimeAvatarClientRoute,
    pub payload: String,
}

impl Debug for RealtimeAvatarClientCommand {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter
            .debug_struct("RealtimeAvatarClientCommand")
            .field("route", &self.route)
            .field("payload_bytes", &self.payload.len())
            .finish()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RealtimeAvatarCapabilities {
    supported: Vec<RealtimeAvatarCapability>,
}

impl RealtimeAvatarCapabilities {
    #[must_use]
    pub fn new(capabilities: impl IntoIterator<Item = RealtimeAvatarCapability>) -> Self {
        let mut supported = Vec::new();
        for capability in capabilities {
            if !supported.contains(&capability) {
                supported.push(capability);
            }
        }
        Self { supported }
    }

    #[must_use]
    pub fn supports(&self, capability: RealtimeAvatarCapability) -> bool {
        self.supported.contains(&capability)
    }
}

pub trait RealtimeAvatarPort: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;
    fn capabilities(&self) -> RealtimeAvatarCapabilities;

    /// Creates one provider realtime-avatar session and returns WebRTC signaling material.
    ///
    /// # Errors
    /// Returns a typed provider failure when session creation is denied or unavailable.
    fn create_session(
        &self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarSession, ProviderError>;

    /// Completes the WebRTC offer/answer negotiation for an existing provider session.
    ///
    /// # Errors
    /// Returns a typed provider failure for invalid/stale sessions or provider errors.
    fn submit_answer(
        &self,
        session: &RealtimeAvatarSession,
        answer: &WebRtcSessionDescription,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError>;

    /// Submits one ICE candidate, including an end-of-candidates marker when candidate is absent.
    ///
    /// # Errors
    /// Returns a typed provider failure for invalid/stale sessions or provider errors.
    fn submit_ice_candidate(
        &self,
        session: &RealtimeAvatarSession,
        candidate: &WebRtcIceCandidate,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError>;

    /// Sends text for avatar rendering through an already negotiated realtime session.
    ///
    /// # Errors
    /// Returns a typed provider failure when input is rejected or the session is unavailable.
    fn speak_text(
        &self,
        session: &RealtimeAvatarSession,
        text: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError>;

    /// Sends an externally hosted audio asset for avatar rendering.
    ///
    /// # Errors
    /// Returns a typed provider failure when the URL is invalid/rejected or the session is unavailable.
    fn speak_audio_url(
        &self,
        session: &RealtimeAvatarSession,
        audio_url: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError>;

    /// Requests immediate provider-side interruption when supported.
    ///
    /// # Errors
    /// The default returns a non-retryable unavailable error for providers without this capability.
    fn interrupt(
        &self,
        _session: &RealtimeAvatarSession,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        Err(ProviderError {
            kind: ProviderErrorKind::Unavailable,
            retryable: false,
        })
    }

    /// Returns browser-side realtime control metadata for this exact provider session.
    ///
    /// This is intentionally session-scoped because capabilities such as Fluent interruption may
    /// depend on the provider's create-session response rather than static adapter support.
    fn client_control(
        &self,
        _session: &RealtimeAvatarSession,
    ) -> Option<RealtimeAvatarClientControl> {
        None
    }

    /// Parses one browser-received provider data-channel message into a normalized event.
    ///
    /// Raw provider payloads are transient inputs only and should not be logged or persisted.
    ///
    /// # Errors
    /// Returns a typed provider error when a recognized message is malformed.
    fn parse_client_event(
        &self,
        _session: &RealtimeAvatarSession,
        _message: &str,
    ) -> Result<Option<RealtimeAvatarClientEvent>, ProviderError> {
        Ok(None)
    }

    /// Builds one provider-specific browser-side text command after canonical runtime
    /// authorization. Providers whose realtime transport is controlled entirely server-side may
    /// leave the default unavailable result.
    ///
    /// # Errors
    /// The default returns a non-retryable unavailable error.
    fn prepare_client_text(
        &self,
        _session: &RealtimeAvatarSession,
        _text: &str,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarClientCommand, ProviderError> {
        Err(ProviderError {
            kind: ProviderErrorKind::Unavailable,
            retryable: false,
        })
    }

    /// Builds one provider-specific browser data-channel interrupt command after canonical runtime
    /// authorization. The returned payload is opaque to the browser and must not be logged.
    ///
    /// # Errors
    /// The default returns a non-retryable unavailable error for providers without client-side
    /// interruption.
    fn prepare_client_interrupt(
        &self,
        _session: &RealtimeAvatarSession,
        _playback_id: Option<&str>,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarClientCommand, ProviderError> {
        Err(ProviderError {
            kind: ProviderErrorKind::Unavailable,
            retryable: false,
        })
    }

    /// Closes an existing provider realtime-avatar session. Cleanup is intentionally independent
    /// of turn cancellation so revoked/cancelled runtime work cannot strand a remote session.
    ///
    /// # Errors
    /// Returns a typed provider failure when close cannot be confirmed.
    fn close_session(&self, session: &RealtimeAvatarSession) -> Result<(), ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webrtc_debug_redacts_signaling_and_turn_secrets() {
        let session = RealtimeAvatarSession {
            provider_resource_id: "stream-secret-123".to_owned(),
            provider_session_id: "session-secret-456".to_owned(),
            transport: RealtimeAvatarTransport::WebRtc {
                offer: WebRtcSessionDescription {
                    kind: "offer".to_owned(),
                    sdp: "v=0 PRIVATE-SDP 10.0.0.7".to_owned(),
                },
                ice_servers: vec![WebRtcIceServer {
                    urls: vec!["turn:private.example:3478?transport=tcp".to_owned()],
                    username: Some("turn-user-secret".to_owned()),
                    credential: Some("turn-password-secret".to_owned()),
                }],
            },
        };
        let candidate = WebRtcIceCandidate {
            candidate: Some("candidate:1 1 UDP 1 10.0.0.8 55555 typ host".to_owned()),
            sdp_mid: Some("sensitive-mid".to_owned()),
            sdp_mline_index: Some(0),
        };

        let ice_server = match &session.transport {
            RealtimeAvatarTransport::WebRtc { ice_servers, .. } => &ice_servers[0],
            RealtimeAvatarTransport::LiveKit { .. } => unreachable!(),
        };
        let debug = format!("{session:?} {candidate:?} {ice_server:?}");
        for secret in [
            "stream-secret-123",
            "session-secret-456",
            "PRIVATE-SDP",
            "10.0.0.7",
            "private.example",
            "turn-user-secret",
            "turn-password-secret",
            "10.0.0.8",
            "sensitive-mid",
        ] {
            assert!(!debug.contains(secret));
        }
        assert!(debug.contains("sdp_bytes"));
        assert!(debug.contains("candidate_bytes"));
        assert!(debug.contains("credential_present"));
    }
}
