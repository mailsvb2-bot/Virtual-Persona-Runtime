//! Canonical D-ID WebRTC and Echo/LiveKit session allocation and compensation.
use crate::protocol::{
    CloseRequest, CreateStreamRequest, CreateStreamResponse, CreateV2SessionResponse,
};
use crate::response_json::decode_json_response;
use crate::{
    DidAgentStreamsAvatar, cancelled, expect_success, invalid_response, map_transport_error,
    policy_denied, unavailable,
};
use vpr_integration::{
    CancellationProbe, ProviderError, RealtimeAvatarPort, RealtimeAvatarSession,
};

impl DidAgentStreamsAvatar {
    pub(super) fn create_webrtc_session(
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
        let response = expect_success(response)?;
        let body: CreateStreamResponse = decode_json_response(response, Some(cancellation))?;
        let client_interrupt = body.fluent && body.interrupt_enabled;
        // The provider can allocate a real sender and return its authenticated
        // identifiers alongside an invalid SDP. Never discard those IDs before
        // attempting compensation: an invalid browser offer is not a STOP.
        let allocated_identity = (body.id.clone(), body.session_id.clone());
        let session: RealtimeAvatarSession = match body.try_into() {
            Ok(session) => session,
            Err(error) => {
                if !allocated_identity.0.trim().is_empty()
                    && !allocated_identity.1.trim().is_empty()
                    && self
                        .delete_webrtc_identity(&allocated_identity.0, &allocated_identity.1)
                        .is_err()
                {
                    // The remote sender may still exist. Never report a
                    // successful cleanup or allow a retry as if it were safe.
                    return Err(unavailable());
                }
                return Err(error);
            }
        };
        // A revoke may arrive while the blocking D-ID CREATE is in flight.
        // Do not publish the resulting viewer offer. Attempt compensating
        // DELETE using the actual returned provider identity; an unsuccessful
        // close is never interpreted as confirmed remote STOP.
        if cancellation.is_cancelled() {
            if self.close_session(&session).is_err() {
                return Err(unavailable());
            }
            return Err(cancelled());
        }
        if client_interrupt {
            if let Err(error) = self.client_control.register_interrupt(&session) {
                if self.close_session(&session).is_err() {
                    return Err(unavailable());
                }
                return Err(error);
            }
        }
        // Registration itself may race a revoke. Do not return a newly
        // published SDP after the cancellation was committed.
        if cancellation.is_cancelled() {
            if self.close_session(&session).is_err() {
                return Err(unavailable());
            }
            return Err(cancelled());
        }
        Ok(session)
    }

    pub(super) fn delete_webrtc_identity(
        &self,
        resource_id: &str,
        session_id: &str,
    ) -> Result<(), ProviderError> {
        if resource_id.trim().is_empty() || session_id.trim().is_empty() {
            return Err(invalid_response());
        }
        let response = self
            .authorized(self.client.delete(self.stream_url(resource_id)?))
            .json(&CloseRequest { session_id })
            .send()
            .map_err(|error| map_transport_error(&error))?;
        expect_success(response).map(|_| ())
    }

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
