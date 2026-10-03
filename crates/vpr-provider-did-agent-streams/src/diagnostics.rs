use vpr_integration::{
    MAX_PROVIDER_JSON_BODY_BYTES, ProviderError, RealtimeAvatarPort, RealtimeAvatarSession,
    read_bounded_provider_body,
};

use super::protocol::{CreateStreamRequest, CreateStreamResponse};
use super::provider_error::{
    DidRuntimeAccessFailure, expect_success, insufficient_credits, invalid_response,
    map_transport_error, policy_denied,
};
use super::{DidAgentStreamsAvatar, DidRuntimeAccessProbe, PresenterLookupError};

fn credits_exhausted(value: &serde_json::Value) -> Option<bool> {
    if let Some(remaining) = value.get("remaining").and_then(serde_json::Value::as_f64) {
        return remaining.is_finite().then_some(remaining <= 0.0);
    }

    let items = value
        .get("credits")
        .and_then(serde_json::Value::as_array)
        .or_else(|| value.as_array())?;
    if items.is_empty() {
        return None;
    }

    let mut total_remaining = 0.0;
    for item in items {
        let remaining = item.get("remaining")?.as_f64()?;
        if !remaining.is_finite() {
            return None;
        }
        total_remaining += remaining;
    }
    Some(total_remaining <= 0.0)
}

impl DidAgentStreamsAvatar {
    /// Verifies that D-ID accepts the configured account API key without touching an Agent.
    ///
    /// The probe uses the read-only account-level `GET /credits` endpoint. It inspects only
    /// recognized non-secret remaining-credit fields so a known exhausted balance can fail before
    /// session creation; unknown valid response shapes are treated only as successful auth.
    ///
    /// # Errors
    /// Preserves 401 versus 403 for safe operator diagnostics and returns typed insufficient-credit
    /// evidence only when the successful account response explicitly proves a zero balance.
    pub fn probe_account_auth_detailed(&self) -> Result<(), DidRuntimeAccessFailure> {
        let mut url = self.base_url.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|()| DidRuntimeAccessFailure::Provider(invalid_response()))?;
            segments.pop_if_empty();
            segments.push("credits");
        }
        let response = self
            .authorized(self.client.get(url))
            .send()
            .map_err(|error| DidRuntimeAccessFailure::Provider(map_transport_error(&error)))?;
        match response.status().as_u16() {
            401 => return Err(DidRuntimeAccessFailure::Unauthorized),
            403 => return Err(DidRuntimeAccessFailure::Forbidden),
            _ => {}
        }

        let response = expect_success(response).map_err(DidRuntimeAccessFailure::Provider)?;
        let bytes = read_bounded_provider_body(response, MAX_PROVIDER_JSON_BODY_BYTES, None)
            .map_err(DidRuntimeAccessFailure::Provider)?;
        let body: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|_| DidRuntimeAccessFailure::Provider(invalid_response()))?;
        if credits_exhausted(&body) == Some(true) {
            return Err(DidRuntimeAccessFailure::Provider(insufficient_credits()));
        }
        Ok(())
    }

    /// Probes the same D-ID access path used by runtime. If agent metadata is forbidden with 403,
    /// it validates the historical RT0 stream endpoint by creating and immediately closing one
    /// legacy WebRTC session. No provider secret is exposed.
    ///
    /// # Errors
    /// Returns the typed provider failure from metadata discovery or the legacy stream endpoint.
    pub fn probe_runtime_access(&self) -> Result<DidRuntimeAccessProbe, ProviderError> {
        self.probe_runtime_access_detailed()
            .map_err(|error| match error {
                DidRuntimeAccessFailure::Unauthorized | DidRuntimeAccessFailure::Forbidden => {
                    policy_denied()
                }
                DidRuntimeAccessFailure::Provider(provider) => provider,
            })
    }

    /// Probes the runtime D-ID access path while preserving HTTP 401 versus 403 for diagnostics.
    ///
    /// # Errors
    /// Returns a D-ID-specific access failure without exposing credentials or response bodies.
    pub fn probe_runtime_access_detailed(
        &self,
    ) -> Result<DidRuntimeAccessProbe, DidRuntimeAccessFailure> {
        match self.presenter_type() {
            Ok(presenter) => Ok(DidRuntimeAccessProbe::Presenter(presenter)),
            Err(PresenterLookupError::Unauthorized) => Err(DidRuntimeAccessFailure::Unauthorized),
            Err(PresenterLookupError::MetadataForbidden) => {
                self.probe_legacy_stream_access()?;
                Ok(DidRuntimeAccessProbe::LegacyStreamFallback)
            }
            Err(PresenterLookupError::Provider(provider)) => {
                Err(DidRuntimeAccessFailure::Provider(provider))
            }
        }
    }

    fn probe_legacy_stream_access(&self) -> Result<(), DidRuntimeAccessFailure> {
        let response = self
            .authorized(
                self.client.post(
                    self.streams_url()
                        .map_err(DidRuntimeAccessFailure::Provider)?,
                ),
            )
            .json(&CreateStreamRequest {
                fluent: self.config.fluent,
            })
            .send()
            .map_err(|error| DidRuntimeAccessFailure::Provider(map_transport_error(&error)))?;
        match response.status().as_u16() {
            401 => return Err(DidRuntimeAccessFailure::Unauthorized),
            403 => return Err(DidRuntimeAccessFailure::Forbidden),
            _ => {}
        }
        let response = expect_success(response).map_err(DidRuntimeAccessFailure::Provider)?;
        let bytes = read_bounded_provider_body(response, MAX_PROVIDER_JSON_BODY_BYTES, None)
            .map_err(DidRuntimeAccessFailure::Provider)?;
        let body: CreateStreamResponse = serde_json::from_slice(&bytes)
            .map_err(|_| DidRuntimeAccessFailure::Provider(invalid_response()))?;
        let session: RealtimeAvatarSession =
            body.try_into().map_err(DidRuntimeAccessFailure::Provider)?;
        self.close_session(&session)
            .map_err(DidRuntimeAccessFailure::Provider)
    }
}
