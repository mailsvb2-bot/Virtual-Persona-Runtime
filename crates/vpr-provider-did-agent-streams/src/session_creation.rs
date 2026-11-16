//! D-ID Echo `LiveKit` session creation: private speaker opens before browser receives viewer grant.
use crate::protocol::CreateV2SessionResponse;
use crate::response_json::decode_json_response;
use crate::{
    DidAgentStreamsAvatar, expect_success, invalid_response, policy_denied, unavailable,
};
use serde_json::Value;
use vpr_integration::{
    CancellationProbe, ProviderError, ProviderErrorKind, RealtimeAvatarSession,
};

impl DidAgentStreamsAvatar {
    fn compensate_v2_create(&self, session_id: Option<&str>) -> Result<(), ProviderError> {
        let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) else {
            // A malformed/late response without a recoverable exact ID cannot
            // be declared clean. Do not let the caller retry an ambiguous POST.
            return Err(unavailable());
        };
        self.delete_v2_session_identity(session_id)
    }

    pub(super) fn create_livekit_session(
        &self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<RealtimeAvatarSession, ProviderError> {
        Self::ensure_active(cancellation)?;
        // A viewer-side LiveKit token cannot be atomically revoked with a
        // browser-published did.speak. No expressive session is issued without
        // a separately credentialed server-owned Echo audio publisher.
        let backend = self.echo_backend.as_ref().ok_or_else(policy_denied)?;
        let response = self
            .authorized(self.client.post(self.v2_sessions_url()?))
            .json(&serde_json::json!({"session_type": "echo"}))
            .send()
            .map_err(|error| {
                if error.is_timeout() {
                    ProviderError {
                        kind: ProviderErrorKind::Timeout,
                        retryable: false,
                    }
                } else {
                    ProviderError {
                        kind: ProviderErrorKind::Unavailable,
                        retryable: false,
                    }
                }
            })?;

        let status = response.status().as_u16();
        if !response.status().is_success() {
            // Retrying an ambiguous POST can create a second billable session.
            // Preserve authentication/policy errors, but never mark a timeout,
            // rate limit or provider 5xx as safely retryable CREATE.
            if status == 408 {
                return Err(ProviderError {
                    kind: ProviderErrorKind::Timeout,
                    retryable: false,
                });
            }
            if status == 429 || (500..=599).contains(&status) {
                return Err(ProviderError {
                    kind: ProviderErrorKind::Unavailable,
                    retryable: false,
                });
            }
            return expect_success(response).map(|_| unreachable!());
        }
        let unexpected_success_status = status != 201;

        // The provider may have allocated a session before the visitor revoke
        // arrives. Read the bounded response without the turn cancellation
        // probe so its exact session ID remains available for compensation.
        let raw: Value = decode_json_response(response, None).map_err(|_| unavailable())?;
        let allocated_id = raw.get("id").and_then(Value::as_str).map(str::to_owned);
        let body: CreateV2SessionResponse = match serde_json::from_value(raw) {
            Ok(body) => body,
            Err(_) => {
                self.compensate_v2_create(allocated_id.as_deref())?;
                return Err(invalid_response());
            }
        };
        let provider_session_id = body.id.clone();
        let session_url = body.session_url.clone();
        let echo_secret = body
            .echo_token
            .as_deref()
            .filter(|token| !token.trim().is_empty())
            .map(str::to_owned);

        if unexpected_success_status {
            self.compensate_v2_create(Some(&provider_session_id))?;
            return Err(invalid_response());
        }

        let session: RealtimeAvatarSession = match body.try_into() {
            Ok(session) => session,
            Err(error) => {
                self.compensate_v2_create(Some(&provider_session_id))?;
                return Err(error);
            }
        };
        let Some(secret) = echo_secret else {
            self.compensate_v2_create(Some(&provider_session_id))?;
            return Err(invalid_response());
        };

        if cancellation.is_cancelled() {
            self.compensate_v2_create(Some(&provider_session_id))?;
            return Err(crate::cancelled());
        }

        // A repeated provider session ID must not tear down an existing
        // Echo publisher. The registry's atomic insert remains the final
        // race fence after this read-only precheck.
        if self
            .echo_sessions
            .contains(&provider_session_id)
            .map_err(|_| unavailable())?
        {
            return Err(invalid_response());
        }

        if let Err(error) = self.echo_sessions.open_session(
            backend.as_ref(),
            &provider_session_id,
            &session_url,
            &secret,
            cancellation,
        ) {
            // open_session retains its only sender identity when local STOP
            // is unconfirmed. Do not discard that state or race a provider
            // DELETE whose acknowledgement could not be durably combined.
            let sender_pending = self
                .echo_sessions
                .contains(&provider_session_id)
                .map_err(|_| unavailable())?;
            if sender_pending {
                return Err(unavailable());
            }
            self.compensate_v2_create(Some(&provider_session_id))?;
            return Err(error);
        }

        if cancellation.is_cancelled() {
            // Close the private publisher before deleting the provider session.
            // If local STOP is uncertain, preserve the registry identity.
            self.echo_sessions
                .close_session(backend.as_ref(), &provider_session_id)
                .map_err(|_| unavailable())?;
            self.compensate_v2_create(Some(&provider_session_id))?;
            return Err(crate::cancelled());
        }

        Ok(session)
    }
}
