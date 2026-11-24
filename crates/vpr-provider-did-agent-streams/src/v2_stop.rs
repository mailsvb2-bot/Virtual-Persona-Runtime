//! D-ID V2 session deletion is owned solely by the canonical provider close path.
//! HTTP 204 proves deletion receipt, NOT termination of both remote RTP senders.
use vpr_integration::{ProviderError, RealtimeAvatarSession};

use super::{DidAgentStreamsAvatar, invalid_response, policy_denied, unavailable};

impl DidAgentStreamsAvatar {
    fn v2_exact_session_url(&self, session_id: &str) -> Result<reqwest::Url, ProviderError> {
        if session_id.is_empty() || session_id.len() > 256 || session_id.chars().any(char::is_control)
        {
            return Err(invalid_response());
        }
        let mut url = self.base_url.clone();
        {
            let mut parts = url.path_segments_mut().map_err(|()| invalid_response())?;
            parts.pop_if_empty();
            parts.extend(["v2", "agents", "sessions"]);
            parts.push(session_id);
        }
        Ok(url)
    }

    /// Single canonical close, following local Echo STOP and optional destructive
    /// vendor session DELETE. The release gate remains closed on any HTTP result.
    pub(super) fn close_livekit_session(
        &self,
        session: &RealtimeAvatarSession,
    ) -> Result<(), ProviderError> {
        // V2 transport conversion binds both identifiers to the exact vendor ID.
        if session.provider_resource_id != session.provider_session_id {
            return Err(policy_denied());
        }
        if let Some(backend) = &self.echo_backend {
            self.echo_sessions
                .close_session(backend.as_ref(), &session.provider_session_id)?;
        }
        // D-ID DELETE permanently erases transcript, insights and recordings.
        // Opt-in is explicit and disabled by default.
        if self.config.v2_delete_on_close {
            let response = self
                .authorized(
                    self.client
                        .delete(self.v2_exact_session_url(&session.provider_session_id)?),
                )
                .send()
                .map_err(|_| unavailable())?;
            // Neither 204, 404 nor timeout proves that both remote audio and
            // video publishers stopped. Treat lost responses as unconfirmed.
            if response.status().as_u16() != 204 {
                return Err(unavailable());
            }
        }
        Err(unavailable())
    }
}
