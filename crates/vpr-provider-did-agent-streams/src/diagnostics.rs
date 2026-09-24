use vpr_integration::{ProviderError, RealtimeAvatarPort, RealtimeAvatarSession};

use super::protocol::{CreateStreamRequest, CreateStreamResponse};
use super::provider_error::{
    DidRuntimeAccessFailure, expect_success, invalid_response, map_transport_error, policy_denied,
};
use super::{DidAgentStreamsAvatar, DidRuntimeAccessProbe, PresenterLookupError};

impl DidAgentStreamsAvatar {
    /// Verifies that D-ID accepts the configured account API key without touching an Agent.
    ///
    /// The probe uses the read-only `GET /tools?limit=1` endpoint and does not inspect or expose
    /// response data.
    ///
    /// # Errors
    /// Preserves 401 versus 403 for safe operator diagnostics.
    pub fn probe_account_auth_detailed(&self) -> Result<(), DidRuntimeAccessFailure> {
        let mut url = self.base_url.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|()| DidRuntimeAccessFailure::Provider(invalid_response()))?;
            segments.pop_if_empty();
            segments.push("tools");
        }
        url.query_pairs_mut().append_pair("limit", "1");

        let response = self
            .authorized(self.client.get(url))
            .send()
            .map_err(|error| DidRuntimeAccessFailure::Provider(map_transport_error(&error)))?;
        match response.status().as_u16() {
            401 => Err(DidRuntimeAccessFailure::Unauthorized),
            403 => Err(DidRuntimeAccessFailure::Forbidden),
            _ => expect_success(response)
                .map(|_| ())
                .map_err(DidRuntimeAccessFailure::Provider),
        }
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
        let body: CreateStreamResponse = response
            .json()
            .map_err(|_| DidRuntimeAccessFailure::Provider(invalid_response()))?;
        let session: RealtimeAvatarSession =
            body.try_into().map_err(DidRuntimeAccessFailure::Provider)?;
        self.close_session(&session)
            .map_err(DidRuntimeAccessFailure::Provider)
    }
}
