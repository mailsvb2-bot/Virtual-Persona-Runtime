//! Exact-session deletion for D-ID Agents Sessions V2.
use crate::{DidAgentStreamsAvatar, expect_success, invalid_response, map_transport_error};
use vpr_integration::{ProviderError, ProviderErrorKind};

impl DidAgentStreamsAvatar {
    /// Builds the documented V2 exact-session resource URL. This is distinct
    /// from the legacy /agents/{agentId}/streams/{streamId} resource.
    fn v2_session_url(&self, session_id: &str) -> Result<reqwest::Url, ProviderError> {
        if session_id.trim().is_empty() {
            return Err(invalid_response());
        }
        let mut url = self.base_url.clone();
        {
            let mut segments = url.path_segments_mut().map_err(|()| invalid_response())?;
            segments.pop_if_empty();
            segments.extend(["v2", "agents", "sessions"]);
            segments.push(session_id);
        }
        Ok(url)
    }

    /// Deletes one exact V2 session. Only the documented 204 response is a
    /// provider deletion receipt; 404 and other 2xx responses remain unknown.
    pub(crate) fn delete_v2_session_identity(&self, session_id: &str) -> Result<(), ProviderError> {
        let response = self
            .authorized(self.client.delete(self.v2_session_url(session_id)?))
            .send()
            .map_err(|error| map_transport_error(&error))?;
        let status = response.status();
        if status.as_u16() == 204 {
            Ok(())
        } else if status.as_u16() == 404 || status.is_success() {
            Err(ProviderError {
                kind: ProviderErrorKind::Unavailable,
                retryable: true,
            })
        } else {
            expect_success(response).map(|_| ())
        }
    }
}
