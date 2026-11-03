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
        max_chunk_bytes: usize,
    ) -> Result<Option<Vec<u8>>, ProviderError> {
        if max_chunk_bytes == 0 {
            return Err(invalid_response());
        }
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
        match result? {
            Some(bytes) if bytes.len() > max_chunk_bytes => Err(invalid_response()),
            Some(bytes) => Ok(Some(bytes.to_vec())),
            None => Ok(None),
        }
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
            let remaining = max_bytes.saturating_sub(body.len());
            let Some(chunk) =
                self.chunk(&mut response, cancellation, remaining.saturating_add(1))?
            else {
                return Ok(body);
            };
            if chunk.len() > remaining {
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

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    struct NeverCancelled;

    impl CancellationProbe for NeverCancelled {
        fn is_cancelled(&self) -> bool {
            false
        }
    }

    #[test]
    fn async_body_reader_rejects_unknown_length_above_hard_cap() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/provider", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            assert!(stream.read(&mut request).unwrap() > 0);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .unwrap();
            stream.write_all(b"c8\r\n").unwrap();
            stream.write_all(&[b'a'; 200]).unwrap();
            stream.write_all(b"\r\n0\r\n\r\n").unwrap();
        });
        let http = CancellableProviderHttp::new(Duration::from_secs(2)).unwrap();
        let response = http
            .send(http.post(url).body("test"), &NeverCancelled)
            .unwrap();
        let error = http
            .read_bounded_body(response, 64, &NeverCancelled)
            .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
        server.join().unwrap();
    }
}
