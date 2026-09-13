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
pub struct RealtimeAvatarSession {
    pub provider_stream_id: String,
    pub provider_session_id: String,
    pub offer: WebRtcSessionDescription,
    pub ice_servers: Vec<WebRtcIceServer>,
}

impl Debug for RealtimeAvatarSession {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter
            .debug_struct("RealtimeAvatarSession")
            .field("stream_id_bytes", &self.provider_stream_id.len())
            .field("session_id_bytes", &self.provider_session_id.len())
            .field("offer", &self.offer)
            .field("ice_server_count", &self.ice_servers.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealtimeAvatarCapability {
    TextInput,
    AudioUrlInput,
    Interrupt,
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

    /// Closes an existing provider realtime-avatar session.
    ///
    /// # Errors
    /// Returns a typed provider failure when close cannot be confirmed.
    fn close_session(
        &self,
        session: &RealtimeAvatarSession,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webrtc_debug_redacts_signaling_and_turn_secrets() {
        let session = RealtimeAvatarSession {
            provider_stream_id: "stream-secret-123".to_owned(),
            provider_session_id: "session-secret-456".to_owned(),
            offer: WebRtcSessionDescription {
                kind: "offer".to_owned(),
                sdp: "v=0 PRIVATE-SDP 10.0.0.7".to_owned(),
            },
            ice_servers: vec![WebRtcIceServer {
                urls: vec!["turn:private.example:3478?transport=tcp".to_owned()],
                username: Some("turn-user-secret".to_owned()),
                credential: Some("turn-password-secret".to_owned()),
            }],
        };
        let candidate = WebRtcIceCandidate {
            candidate: Some("candidate:1 1 UDP 1 10.0.0.8 55555 typ host".to_owned()),
            sdp_mid: Some("sensitive-mid".to_owned()),
            sdp_mline_index: Some(0),
        };

        let debug = format!("{session:?} {candidate:?} {:?}", session.ice_servers[0]);
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
