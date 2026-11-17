//! D-ID Echo `LiveKit` session creation: private speaker opens before browser receives viewer grant.
use crate::protocol::CreateV2SessionResponse;
use crate::response_json::decode_json_response;
use crate::{DidAgentStreamsAvatar, expect_success, invalid_response, policy_denied};
use vpr_integration::{CancellationProbe, ProviderError, ProviderErrorKind, RealtimeAvatarSession};

impl DidAgentStreamsAvatar {
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
                // A timed-out CREATE may already have allocated a billable
                // provider session. V2 remote STOP is not verified, so this
                // ambiguous operation must never be retried as a fresh CREATE.
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
            // A provider may allocate before returning 408/429/5xx. Preserve
            // the uncertainty and prohibit automatic retry/duplicate CREATE.
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
        if status != 201 {
            return Err(invalid_response());
        }

        let body: CreateV2SessionResponse = decode_json_response(response, Some(cancellation))
            .map_err(|error| ProviderError {
                kind: error.kind,
                retryable: false,
            })?;
        let secret = body
            .echo_token
            .as_deref()
            .filter(|token| !token.trim().is_empty())
            .ok_or_else(invalid_response)?
            .to_owned();
        let session_url = body.session_url.clone();
        let session: RealtimeAvatarSession =
            body.try_into()
                .map_err(|error: ProviderError| ProviderError {
                    kind: error.kind,
                    retryable: false,
                })?;
        self.echo_sessions
            .open_session(
                backend.as_ref(),
                &session.provider_session_id,
                &session_url,
                &secret,
                cancellation,
            )
            .map_err(|error| ProviderError {
                kind: error.kind,
                retryable: false,
            })?;
        Ok(session)
    }
}
