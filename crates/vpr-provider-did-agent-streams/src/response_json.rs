//! Bounded provider JSON decoder shared by D-ID control-plane operations.
use reqwest::blocking::Response;
use serde::de::DeserializeOwned;
use vpr_integration::{
    CancellationProbe, MAX_PROVIDER_JSON_BODY_BYTES, ProviderError, ProviderErrorKind,
    read_bounded_provider_body,
};

pub(super) fn decode_json_response<T: DeserializeOwned>(
    response: Response,
    cancellation: Option<&dyn CancellationProbe>,
) -> Result<T, ProviderError> {
    let body = read_bounded_provider_body(response, MAX_PROVIDER_JSON_BODY_BYTES, cancellation)?;
    serde_json::from_slice(&body).map_err(|_| ProviderError {
        kind: ProviderErrorKind::InvalidResponse,
        retryable: false,
    })
}
