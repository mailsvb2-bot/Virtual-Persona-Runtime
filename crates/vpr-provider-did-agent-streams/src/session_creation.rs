//! D-ID Echo LiveKit session creation: private speaker opens before browser receives viewer grant.
use crate::{
    DidAgentStreamsAvatar, invalid_response, policy_denied,
    map_transport_error, expect_success,
};
use crate::protocol::CreateV2SessionResponse;
use crate::response_json::decode_json_response;
use vpr_integration::{CancellationProbe, ProviderError, RealtimeAvatarSession};

impl DidAgentStreamsAvatar {
    pub(super) fn create_livekit_session(
        &self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarSession, ProviderError> {
        Self::ensure_active(cancellation)?;
        // A viewer-side LiveKit token cannot be atomically revoked with a
        // browser-published did.speak. No expressive session is issued without
        // a separately credentialed server-owned Echo audio publisher.
        if self.echo_backend.is_none() {
            return Err(policy_denied());
        }
        let request = self.authorized(self.client.post(self.v2_sessions_url()?));
        let request = if self.echo_backend.is_some() {
            request.json(&serde_json::json!({"session_type": "echo"}))
        } else {
            request
        };
        let response = request
            .send()
            .map_err(|error| map_transport_error(&error))?;
        let response = expect_success(response)?;
        let body: CreateV2SessionResponse = decode_json_response(response, Some(cancellation))?;
        if let Some(backend) = &self.echo_backend {
            let secret = body
                .echo_token
                .as_deref()
                .filter(|token| !token.trim().is_empty())
                .ok_or_else(invalid_response)?
                .to_owned();
            let session_url = body.session_url.clone();
            let session: RealtimeAvatarSession = body.try_into()?;
            self.echo_sessions.open_session(
                backend.as_ref(),
                &session.provider_session_id,
                &session_url,
                &secret,
                cancellation,
            )?;
            Ok(session)
        } else {
            body.try_into()
        }
    }

}
