use std::io::{BufRead, Read};
use std::time::Duration;

use reqwest::blocking::{Client, Response};

use crate::{CancellationProbe, ProviderError, ProviderErrorKind};

/// Maximum accepted JSON/control-plane provider response body.
pub const MAX_PROVIDER_JSON_BODY_BYTES: usize = 2 * 1024 * 1024;
/// Maximum accepted binary provider response body (for example synthesized audio).
pub const MAX_PROVIDER_BINARY_BODY_BYTES: usize = 16 * 1024 * 1024;
/// Maximum bytes accepted in one provider streaming protocol line/event.
pub const MAX_PROVIDER_STREAM_LINE_BYTES: usize = 256 * 1024;

/// Builds the canonical blocking HTTP client used by provider adapters.
///
/// Provider endpoints are validated by each adapter before requests are built. Redirects are
/// deliberately disabled here so an accepted endpoint cannot redirect credentials or request
/// bodies to a second destination that never passed that endpoint policy.
///
/// # Errors
/// Returns reqwest's client-construction error.
pub fn build_provider_http_client(timeout: Duration) -> Result<Client, reqwest::Error> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
}

/// Reads one provider response body without ever retaining more than `max_bytes`.
///
/// A declared Content-Length above the limit is rejected early, but observed bytes remain the
/// authority so chunked/unknown-length responses cannot bypass the cap.
///
/// # Errors
/// Returns a typed provider error for cancellation, transport failure, invalid zero limit, or
/// a body that exceeds the hard bound.
pub fn read_bounded_provider_body(
    mut response: Response,
    max_bytes: usize,
    cancellation: Option<&dyn CancellationProbe>,
) -> Result<Vec<u8>, ProviderError> {
    if max_bytes == 0 {
        return Err(invalid_response());
    }
    if response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .is_some_and(|length| length > max_bytes)
    {
        return Err(invalid_response());
    }

    let mut output = Vec::with_capacity(
        response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or(0)
            .min(max_bytes),
    );
    let mut buffer = [0_u8; 8_192];

    loop {
        if cancellation.is_some_and(CancellationProbe::is_cancelled) {
            return Err(cancelled());
        }
        let remaining = max_bytes.saturating_sub(output.len());
        let probe_len = remaining.saturating_add(1).min(buffer.len()).max(1);
        let read = response
            .read(&mut buffer[..probe_len])
            .map_err(|_| unavailable())?;
        if read == 0 {
            return Ok(output);
        }
        if read > remaining {
            return Err(invalid_response());
        }
        output.extend_from_slice(&buffer[..read]);
    }
}

/// Bounded newline framing for provider streaming protocols.
///
/// Unlike `BufRead::lines()`, this reader never grows one frame past `max_line_bytes`.
/// The returned line excludes LF and an optional preceding CR.
///
/// Cancellation is checked between every bounded buffer refill.
///
/// # Errors
/// Returns a typed provider error when a frame is too large, non-UTF-8, unreadable, or cancelled.
pub struct BoundedProviderLineReader<R> {
    reader: R,
    max_line_bytes: usize,
    pending: Vec<u8>,
}

impl<R: BufRead> BoundedProviderLineReader<R> {
    #[must_use]
    pub fn new(reader: R, max_line_bytes: usize) -> Self {
        Self {
            reader,
            max_line_bytes,
            pending: Vec::new(),
        }
    }

    pub fn next_line(
        &mut self,
        cancellation: &dyn CancellationProbe,
    ) -> Result<Option<String>, ProviderError> {
        if self.max_line_bytes == 0 {
            return Err(invalid_response());
        }
        self.pending.clear();

        loop {
            if cancellation.is_cancelled() {
                return Err(cancelled());
            }

            let available = self.reader.fill_buf().map_err(|_| unavailable())?;
            if available.is_empty() {
                if self.pending.is_empty() {
                    return Ok(None);
                }
                return self.finish_line().map(Some);
            }

            if let Some(newline) = available.iter().position(|byte| *byte == b'\n') {
                if self
                    .pending
                    .len()
                    .checked_add(newline)
                    .is_none_or(|length| length > self.max_line_bytes)
                {
                    return Err(invalid_response());
                }
                self.pending.extend_from_slice(&available[..newline]);
                self.reader.consume(newline + 1);
                return self.finish_line().map(Some);
            }

            if self
                .pending
                .len()
                .checked_add(available.len())
                .is_none_or(|length| length > self.max_line_bytes)
            {
                return Err(invalid_response());
            }
            let consumed = available.len();
            self.pending.extend_from_slice(available);
            self.reader.consume(consumed);
        }
    }

    fn finish_line(&mut self) -> Result<String, ProviderError> {
        if self.pending.last() == Some(&b'\r') {
            self.pending.pop();
        }
        String::from_utf8(std::mem::take(&mut self.pending)).map_err(|_| invalid_response())
    }
}

const fn invalid_response() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::InvalidResponse,
        retryable: false,
    }
}

const fn unavailable() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::Unavailable,
        retryable: true,
    }
}

const fn cancelled() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::Cancelled,
        retryable: false,
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    use reqwest::header::AUTHORIZATION;

    use super::{
        BoundedProviderLineReader, build_provider_http_client, read_bounded_provider_body,
    };
    use crate::{CancellationProbe, ProviderErrorKind};

    struct NeverCancelled;
    impl CancellationProbe for NeverCancelled {
        fn is_cancelled(&self) -> bool {
            false
        }
    }

    struct AlwaysCancelled;
    impl CancellationProbe for AlwaysCancelled {
        fn is_cancelled(&self) -> bool {
            true
        }
    }

    fn serve_once(status: &'static str, extra_headers: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).unwrap();
            let response = format!(
                "HTTP/1.1 {status}\r\n{extra_headers}Content-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        format!("http://{address}/provider")
    }

    fn serve_body(body: Vec<u8>, declared_length: bool, chunked: bool) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).unwrap();
            if chunked {
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n")
                    .unwrap();
                for chunk in body.chunks(3) {
                    write!(stream, "{:x}\r\n", chunk.len()).unwrap();
                    stream.write_all(chunk).unwrap();
                    stream.write_all(b"\r\n").unwrap();
                }
                stream.write_all(b"0\r\n\r\n").unwrap();
            } else {
                let header = if declared_length {
                    format!("Content-Length: {}\r\n", body.len())
                } else {
                    String::new()
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\n{header}Connection: close\r\n\r\n"
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        format!("http://{address}/provider")
    }

    #[test]
    fn direct_loopback_request_remains_supported() {
        let endpoint = serve_once("204 No Content", String::new());
        let response = build_provider_http_client(Duration::from_secs(2))
            .unwrap()
            .get(endpoint)
            .send()
            .unwrap();
        assert_eq!(response.status().as_u16(), 204);
    }

    #[test]
    fn redirects_are_returned_without_following_or_leaking_authorization() {
        for status in ["307 Temporary Redirect", "308 Permanent Redirect"] {
            let target = TcpListener::bind("127.0.0.1:0").unwrap();
            target.set_nonblocking(true).unwrap();
            let target_url = format!("http://{}/capture", target.local_addr().unwrap());
            let endpoint = serve_once(status, format!("Location: {target_url}\r\n"));

            let response = build_provider_http_client(Duration::from_secs(2))
                .unwrap()
                .post(endpoint)
                .header(AUTHORIZATION, "Bearer must-not-cross-origin")
                .body("sensitive-body")
                .send()
                .unwrap();

            assert_eq!(
                response.status().as_u16(),
                if status.starts_with("307") { 307 } else { 308 }
            );
            thread::sleep(Duration::from_millis(25));
            match target.accept() {
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Ok(_) => panic!("redirect target received a request despite no-redirect policy"),
                Err(error) => panic!("unexpected redirect-target accept error: {error}"),
            }
        }
    }

    #[test]
    fn bounded_body_accepts_exact_limit_and_rejects_declared_or_chunked_overflow() {
        let client = build_provider_http_client(Duration::from_secs(2)).unwrap();

        let exact = client
            .get(serve_body(vec![7; 16], true, false))
            .send()
            .unwrap();
        assert_eq!(
            read_bounded_provider_body(exact, 16, Some(&NeverCancelled)).unwrap(),
            vec![7; 16]
        );

        let declared = client
            .get(serve_body(vec![8; 17], true, false))
            .send()
            .unwrap();
        assert_eq!(
            read_bounded_provider_body(declared, 16, Some(&NeverCancelled))
                .unwrap_err()
                .kind,
            ProviderErrorKind::InvalidResponse
        );

        let chunked = client
            .get(serve_body(vec![9; 17], false, true))
            .send()
            .unwrap();
        assert_eq!(
            read_bounded_provider_body(chunked, 16, Some(&NeverCancelled))
                .unwrap_err()
                .kind,
            ProviderErrorKind::InvalidResponse
        );
    }

    #[test]
    fn bounded_body_cancellation_wins_before_reading() {
        let client = build_provider_http_client(Duration::from_secs(2)).unwrap();
        let response = client
            .get(serve_body(vec![1; 8], true, false))
            .send()
            .unwrap();
        assert_eq!(
            read_bounded_provider_body(response, 16, Some(&AlwaysCancelled))
                .unwrap_err()
                .kind,
            ProviderErrorKind::Cancelled
        );
    }

    #[test]
    fn bounded_line_reader_accepts_exact_boundary_and_rejects_one_byte_over() {
        let mut exact = BoundedProviderLineReader::new(
            BufReader::with_capacity(3, &b"12345678\n"[..]),
            8,
        );
        assert_eq!(
            exact.next_line(&NeverCancelled).unwrap().as_deref(),
            Some("12345678")
        );
        assert_eq!(exact.next_line(&NeverCancelled).unwrap(), None);

        let mut over = BoundedProviderLineReader::new(
            BufReader::with_capacity(2, &b"123456789\n"[..]),
            8,
        );
        assert_eq!(
            over.next_line(&NeverCancelled).unwrap_err().kind,
            ProviderErrorKind::InvalidResponse
        );
    }

    #[test]
    fn bounded_line_reader_checks_cancellation_between_refills() {
        let mut reader =
            BoundedProviderLineReader::new(BufReader::new(&b"data: never read\n"[..]), 64);
        assert_eq!(
            reader.next_line(&AlwaysCancelled).unwrap_err().kind,
            ProviderErrorKind::Cancelled
        );
    }
}
