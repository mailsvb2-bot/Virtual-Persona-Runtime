//! Exact-session D-ID V2 closure, called ONLY from the canonical avatar STOP path.
//! An optional destructive DELETE receipt is never evidence of dual-track RTP STOP.
use vpr_integration::{ProviderError, RealtimeAvatarSession};

use super::{DidAgentStreamsAvatar, invalid_response, policy_denied, unavailable};

fn exact_session_url(
    adapter: &DidAgentStreamsAvatar,
    session_id: &str,
) -> Result<reqwest::Url, ProviderError> {
    if session_id.is_empty() || session_id.len() > 256 || session_id.chars().any(char::is_control) {
        return Err(invalid_response());
    }
    let mut url = adapter.base_url.clone();
    {
        let mut parts = url.path_segments_mut().map_err(|()| invalid_response())?;
        parts.pop_if_empty();
        parts.extend(["v2", "agents", "sessions"]);
        parts.push(session_id);
    }
    Ok(url)
}

/// Reuses the single canonical close owner, not another sender or cleanup path.
/// Return unconfirmed even after authenticated HTTP 204: only an independent
/// provider/receiver observation can prove that both remote publishers stopped.
pub(super) fn close_v2(
    adapter: &DidAgentStreamsAvatar,
    session: &RealtimeAvatarSession,
) -> Result<(), ProviderError> {
    if session.provider_resource_id != session.provider_session_id {
        return Err(policy_denied());
    }

    if let Some(backend) = &adapter.echo_backend {
        adapter
            .echo_sessions
            .close_session(backend.as_ref(), &session.provider_session_id)?;
    }

    // D-ID DELETE removes transcripts/insights/recordings; default is disabled.
    if adapter.config.v2_delete_on_close {
        let response = adapter
            .authorized(
                adapter
                    .client
                    .delete(exact_session_url(adapter, &session.provider_session_id)?),
            )
            .send()
            .map_err(|_| unavailable())?;
        // A lost reply followed by 404 is ambiguous: do not treat as success.
        if response.status().as_u16() != 204 {
            return Err(unavailable());
        }
    }
    Err(unavailable())
}
