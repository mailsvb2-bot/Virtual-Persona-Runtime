use std::time::Duration;

use crate::provider_http::{
    build_provider_async_http_client, cancelled, invalid_response, unavailable,
};
use crate::{CancellationProbe, ProviderError, ProviderErrorKind};

/// One canonical cancellable HTTP transport shared by RT0 and RT1 provider
/// adapters. Cancellation drops the in-flight request/body future; it does
/// not create a detached worker or grant permission to publish provider output.
pub struct CancellableProviderHttp {
    client: reqwest::Client,
    runtime: tokio::runtime::Runtime,
}

impl CancellableProviderHttp {
    /// # Errors
    /// Refuses invalid transport configuration without exposing credentials.
    pub fn new(timeout: Duration) -> Result<Self, ProviderError> {
        let client = build_provider_async_http_client(timeout)
            .map_err(|error| async_transport_error(&error))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| unavailable())?;
        Ok(Self { client, runtime })
    }

    #[must_use]
    pub fn post(&self, url: impl reqwest::IntoUrl) -> reqwest::RequestBuilder {
        self.client.post(url)
    }

    /// A revoked turn cannot start an HTTP operation. If revocation happens
    /// while awaiting response headers, the request future is dropped.
    /// # Errors
    /// Returns typed cancellation, timeout or transport failure.
    pub fn send(
        &self,
        request: reqwest::RequestBuilder,
        cancellation: &dyn CancellationProbe,
    ) -> Result<reqwest::Response, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let result = self.runtime.block_on(async {
            let mut send = Box::pin(request.send());
            loop {
                match tokio::time::timeout(Duration::from_millis(20), &mut send).await {
                    Ok(response) => break response.map_err(|error| async_transport_error(&error)),
                    Err(_) if cancellation.is_cancelled() => return Err(cancelled()),
                    Err(_) => {}
                }
            }
        });
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        result
    }

    /// Abort a stalled response-body read on revocation, rather than waiting
    /// for a provider-controlled HTTP timeout. Never return late bytes.
    /// # Errors
    /// Returns a typed provider error on cancellation or network failure.
    pub fn chunk(
        &self,
        response: &mut reqwest::Response,
        cancellation: &dyn CancellationProbe,
    ) -> Result<Option<Vec<u8>>, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let result = self.runtime.block_on(async {
            let mut chunk = Box::pin(response.chunk());
            loop {
                match tokio::time::timeout(Duration::from_millis(20), &mut chunk).await {
                    Ok(chunk) => break chunk.map_err(|error| async_transport_error(&error)),
                    Err(_) if cancellation.is_cancelled() => return Err(cancelled()),
                    Err(_) => {}
                }
            }
        });
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        result.map(|chunk| chunk.map(|bytes| bytes.to_vec()))
    }

    /// Read JSON/control data with both a declared and observed byte cap.
    /// # Errors
    /// Refuses zero bounds, oversized responses and revoked operations.
    pub fn read_bounded_body(
        &self,
        mut response: reqwest::Response,
        max_bytes: usize,
        cancellation: &dyn CancellationProbe,
    ) -> Result<Vec<u8>, ProviderError> {
        if max_bytes == 0
            || response
                .content_length()
                .is_some_and(|length| length > u64::try_from(max_bytes).unwrap_or(u64::MAX))
        {
            return Err(invalid_response());
        }
        let mut body = Vec::new();
        loop {
            let Some(chunk) = self.chunk(&mut response, cancellation)? else {
                return Ok(body);
            };
            if chunk.len() > max_bytes.saturating_sub(body.len()) {
                return Err(invalid_response());
            }
            body.extend_from_slice(&chunk);
        }
    }
}

fn async_transport_error(error: &reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError {
            kind: ProviderErrorKind::Timeout,
            retryable: true,
        }
    } else {
        unavailable()
    }
}
