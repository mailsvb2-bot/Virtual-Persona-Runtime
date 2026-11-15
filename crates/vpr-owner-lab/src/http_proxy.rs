//! Public localhost TCP owner for the existing `tiny_http` application.
//! RT1 browser responses keep their actual socket registered with the same
//! runtime session that revokes provider egress; no second HTTP router exists.
use super::{AppState, HTTP_WORKERS, http_rt1::Rt1HttpState};
use std::io::{self, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use vpr_runtime::CancellableTcpWriter;

pub(super) const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
pub(super) const RESPONSE_DEADLINE: Duration = Duration::from_secs(90);
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(10);
pub(super) const UPSTREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const UPSTREAM_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
pub(super) const RT1_READ_POLL: Duration = Duration::from_millis(100);

#[path = "http_proxy_response.rs"]
mod response;
#[cfg(test)]
#[path = "http_proxy_rt1_response_tests.rs"]
mod response_tests;
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

pub(super) fn serve(
    listener: &TcpListener,
    upstream: SocketAddr,
    state: &Arc<AppState>,
) -> io::Result<()> {
    let (sender, receiver) = mpsc::sync_channel::<TcpStream>(64);
    let receiver = Arc::new(Mutex::new(receiver));
    for _ in 0..HTTP_WORKERS {
        let receiver = Arc::clone(&receiver);
        let state = Arc::clone(state);
        thread::spawn(move || {
            loop {
                let next = receiver.lock().map(|queue| queue.recv());
                let Ok(Ok(stream)) = next else {
                    break;
                };
                proxy_connection(stream, upstream, &state.rt1);
            }
        });
    }
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                if sender.send(stream).is_err() {
                    return Ok(());
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn proxy_connection(mut stream: TcpStream, upstream: SocketAddr, rt1: &Rt1HttpState) {
    let _ = stream.set_read_timeout(Some(REQUEST_READ_TIMEOUT));
    let (raw_request, method, path) = match read_request(&mut stream) {
        Ok(request) => request,
        Err(error) => {
            eprintln!(
                "owner-lab HTTP proxy request parse failed kind={:?}",
                error.kind()
            );
            write_error(&mut stream, 400, "BAD_REQUEST");
            return;
        }
    };
    let rt1_post = method == "POST"
        && path
            .split('?')
            .next()
            .is_some_and(|value| value.starts_with("/api/rt1/"));
    let request_id =
        rt1_post.then(|| format!("rt1-{}", NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)));
    let forwarded = match rewrite_request(&raw_request, request_id.as_deref()) {
        Ok(request) => request,
        Err(error) => {
            log_proxy_error("request normalization failed", &method, &path, &error);
            write_error(&mut stream, 400, "BAD_REQUEST");
            return;
        }
    };

    if let Some(id) = request_id.as_deref() {
        let (mut writer, cancellation) = match CancellableTcpWriter::new(stream, RESPONSE_DEADLINE)
        {
            Ok(writer) => writer,
            Err(error) => {
                log_proxy_error("RT1 writer creation failed", &method, &path, &error);
                return;
            }
        };
        if rt1
            .register_transport_writer(id.to_owned(), cancellation.clone())
            .is_err()
        {
            let _ =
                writer.write_all_bounded(&error_response(503, "RT1_RESPONSE_WRITER_UNAVAILABLE"));
            return;
        }
        let delivery = relay_rt1_response(upstream, &forwarded, &mut writer, &cancellation);
        let delivered = delivery.is_ok();
        if let Err(error) = delivery {
            log_proxy_error("RT1 response relay failed", &method, &path, &error);
        }
        rt1.finish_transport_delivery(id, delivered);
    } else {
        match response::relay_ordinary_response(upstream, &forwarded, &mut stream) {
            Ok(status) if status >= 400 => {
                let path = path.split('?').next().unwrap_or("/");
                eprintln!(
                    "owner-lab HTTP proxy upstream status method={method} path={path} status={status}"
                );
            }
            Ok(_) => {}
            Err(error) => log_proxy_error("response relay failed", &method, &path, &error),
        }
    }
}

fn log_proxy_error(stage: &str, method: &str, path: &str, error: &io::Error) {
    let path = path.split('?').next().unwrap_or("/");
    eprintln!(
        "owner-lab HTTP proxy {stage} method={method} path={path} kind={:?}",
        error.kind()
    );
}

fn read_request(stream: &mut TcpStream) -> io::Result<(Vec<u8>, String, String)> {
    let mut reader = BufReader::new(stream);
    let mut header = Vec::with_capacity(1024);
    let mut byte = [0_u8; 1];
    loop {
        if header.len() >= MAX_HEADER_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP headers too large",
            ));
        }
        reader.read_exact(&mut byte)?;
        header.push(byte[0]);
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let (method, path, body_length) = parse_request_header(&header)?;
    let mut body = vec![0_u8; body_length];
    reader.read_exact(&mut body)?;
    let mut request = header;
    request.extend_from_slice(&body);
    Ok((request, method, path))
}

fn parse_request_header(header: &[u8]) -> io::Result<(String, String, usize)> {
    let text = std::str::from_utf8(header)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP header text"))?;
    if !text.ends_with("\r\n\r\n") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "incomplete HTTP headers",
        ));
    }
    let mut lines = text[..text.len() - 4]
        .split("\r\n")
        .filter(|line| !line.is_empty());
    let request_line = lines
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request line"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing method"))?;
    let path = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request path"))?;
    let version = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP version"))?;
    if parts.next().is_some()
        || version != "HTTP/1.1"
        || !path.starts_with('/')
        || method.is_empty()
        || !method.bytes().all(|byte| byte.is_ascii_uppercase())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid HTTP request line",
        ));
    }
    let mut content_length = None;
    let mut host_count = 0;
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP header"))?;
        if name.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "empty HTTP header name",
            ));
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate content length",
                ));
            }
            content_length = Some(value.trim().parse::<usize>().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid content length")
            })?);
        }
        if name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("expect")
            || name.eq_ignore_ascii_case("upgrade")
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported HTTP framing",
            ));
        }
        if name.eq_ignore_ascii_case("host") {
            host_count += 1;
        }
    }
    if host_count != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "one Host header required",
        ));
    }
    let body_length = content_length.unwrap_or(0);
    if body_length > MAX_BODY_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "HTTP body too large",
        ));
    }
    Ok((method.to_owned(), path.to_owned(), body_length))
}

fn rewrite_request(raw: &[u8], transport_id: Option<&str>) -> io::Result<Vec<u8>> {
    let header_end = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "incomplete HTTP request"))?;
    let header = std::str::from_utf8(&raw[..header_end])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP header text"))?;
    let mut lines = header.split("\r\n").filter(|line| !line.is_empty());
    let request_line = lines
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request line"))?;
    let mut forwarded = Vec::with_capacity(raw.len() + 128);
    forwarded.extend_from_slice(request_line.as_bytes());
    forwarded.extend_from_slice(b"\r\n");
    for line in lines {
        let name = line.split_once(':').map_or("", |(name, _)| name.trim());
        if name.eq_ignore_ascii_case("connection")
            || name.eq_ignore_ascii_case("x-vpr-rt1-transport-id")
        {
            continue;
        }
        forwarded.extend_from_slice(line.as_bytes());
        forwarded.extend_from_slice(b"\r\n");
    }
    forwarded.extend_from_slice(b"Connection: close\r\n");
    if let Some(id) = transport_id {
        forwarded.extend_from_slice(format!("X-VPR-RT1-Transport-Id: {id}\r\n").as_bytes());
    }
    forwarded.extend_from_slice(b"\r\n");
    forwarded.extend_from_slice(&raw[header_end..]);
    Ok(forwarded)
}

fn relay_rt1_response(
    upstream_address: SocketAddr,
    request: &[u8],
    writer: &mut CancellableTcpWriter,
    cancellation: &vpr_runtime::TcpWriteRevocation,
) -> io::Result<()> {
    let mut upstream = TcpStream::connect_timeout(&upstream_address, UPSTREAM_CONNECT_TIMEOUT)?;
    upstream.set_write_timeout(Some(UPSTREAM_WRITE_TIMEOUT))?;
    upstream.set_read_timeout(Some(RT1_READ_POLL))?;
    upstream.write_all(request)?;
    upstream.shutdown(Shutdown::Write)?;

    // The internal tiny_http response is a bounded in-memory body. Validate
    // its framing before treating the browser delivery as complete: EOF after
    // a partial Content-Length must trigger the existing revoke/STOP
    // compensation instead of being reported as a successful delivery.
    let deadline = Instant::now() + RESPONSE_DEADLINE;
    let mut header = Vec::with_capacity(1024);
    let mut byte = [0_u8; 1];
    loop {
        read_rt1_response_exact(&mut upstream, &mut byte, deadline, cancellation)?;
        header.push(byte[0]);
        if header.len() > MAX_HEADER_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "RT1 response headers too large",
            ));
        }
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    response::relay_revocable_response_body(
        &mut upstream,
        &header,
        deadline,
        cancellation,
        |bytes| writer.write_all_bounded(bytes),
    )
}

fn read_rt1_response_exact(
    stream: &mut TcpStream,
    output: &mut [u8],
    deadline: Instant,
    cancellation: &vpr_runtime::TcpWriteRevocation,
) -> io::Result<()> {
    let mut offset = 0;
    while offset < output.len() {
        if cancellation.is_revoked() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "RT1 response revoked",
            ));
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "HTTP response deadline exceeded",
            ));
        }
        match stream.read(&mut output[offset..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "RT1 response headers truncated",
                ));
            }
            Ok(count) => offset += count,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::WouldBlock
                ) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn write_error(stream: &mut TcpStream, status: u16, code: &str) {
    let body = error_response(status, code);
    let _ = stream.write_all(&body);
}

fn error_response(status: u16, code: &str) -> Vec<u8> {
    let reason = match status {
        400 => "Bad Request",
        503 => "Service Unavailable",
        _ => "Bad Gateway",
    };
    let body = format!(r#"{{"ok":false,"code":"{code}"}}"#);
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::{parse_request_header, rewrite_request};

    #[test]
    fn proxy_injects_unspoofable_transport_identity_and_closes_upstream() {
        let raw = b"POST /api/rt1/visitor/text HTTP/1.1\r\nHost: 127.0.0.1:8787\r\nOrigin: http://127.0.0.1:8787\r\nConnection: keep-alive\r\nX-VPR-RT1-Transport-Id: spoof\r\nContent-Length: 2\r\n\r\n{}";
        let forwarded = rewrite_request(raw, Some("rt1-9")).unwrap();
        let header_end = forwarded
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap();
        let header = std::str::from_utf8(&forwarded[..header_end]).unwrap();
        assert_eq!(header.matches("X-VPR-RT1-Transport-Id").count(), 1);
        assert!(header.contains("X-VPR-RT1-Transport-Id: rt1-9"));
        assert!(header.contains("Connection: close"));
        assert!(forwarded.ends_with(b"{}"));
    }

    #[test]
    fn proxy_accepts_the_canonical_thirty_second_rt1_microphone_body() {
        // 960,000 PCM bytes encoded as a JSON u8 array can exceed 3 MiB.
        let header = "POST /api/rt1/visitor/voice HTTP/1.1\r\nHost: localhost:8787\r\nContent-Length: 3844096\r\n\r\n";
        assert_eq!(
            parse_request_header(header.as_bytes()).unwrap().2,
            3_844_096
        );
    }

    #[test]
    fn real_rt1_public_socket_closes_when_registered_writer_is_revoked_mid_response() {
        use super::{Rt1HttpState, proxy_connection};
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};
        use std::sync::{Arc, mpsc};
        use std::thread;
        use std::time::Duration;

        let upstream_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_address = upstream_listener.local_addr().unwrap();
        let (transport_id_tx, transport_id_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let upstream_thread = thread::spawn(move || {
            let (mut upstream, _) = upstream_listener.accept().unwrap();
            upstream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            upstream.read_to_end(&mut request).unwrap();
            let transport_id = request
                .split(|byte| *byte == b'\n')
                .find_map(|line| {
                    let line = std::str::from_utf8(line).ok()?.trim_end_matches('\r');
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("X-VPR-RT1-Transport-Id")
                        .then(|| value.trim().to_owned())
                })
                .expect("public proxy injects transport identity");
            transport_id_tx.send(transport_id).unwrap();
            upstream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\nConnection: close\r\n\r\npartial",
                )
                .unwrap();
            let _ = release_rx.recv_timeout(Duration::from_secs(2));
            let _ = upstream.write_all(b"-tail");
        });

        let public_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut browser = TcpStream::connect(public_listener.local_addr().unwrap()).unwrap();
        let (public_socket, _) = public_listener.accept().unwrap();
        browser
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let rt1 = Arc::new(Rt1HttpState::default());
        let worker_state = Arc::clone(&rt1);
        let proxy_thread =
            thread::spawn(move || proxy_connection(public_socket, upstream_address, &worker_state));

        browser
            .write_all(
                b"POST /api/rt1/visitor/text HTTP/1.1\r\nHost: 127.0.0.1:18789\r\nContent-Length: 2\r\n\r\n{}",
            )
            .unwrap();
        let transport_id = transport_id_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("proxy forwards RT1 request");
        let mut response = Vec::new();
        let mut buffer = [0_u8; 1024];
        while !response.windows(7).any(|window| window == b"partial") {
            let count = browser.read(&mut buffer).unwrap();
            assert_ne!(count, 0, "partial response reached the public socket");
            response.extend_from_slice(&buffer[..count]);
        }
        assert!(response.starts_with(b"HTTP/1.1 200 OK"));
        // This is the same handle installed by proxy_connection before it
        // forwarded the request to tiny_http's internal listener.
        let writer = rt1
            .take_transport_writer(&transport_id)
            .expect("public response writer is registered");
        writer.revoke();
        release_tx.send(()).unwrap();
        proxy_thread.join().unwrap();
        upstream_thread.join().unwrap();

        let mut tail = [0_u8; 16];
        match browser.read(&mut tail) {
            Ok(0) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::TimedOut
                ) => {}
            other => panic!("revoked public socket remained writable: {other:?}"),
        }
        assert!(!response.windows(5).any(|window| window == b"-tail"));
        assert!(rt1.take_transport_writer(&transport_id).is_none());
    }

    #[test]
    fn proxy_rejects_ambiguous_or_unbounded_request_framing() {
        for header in [
            b"POST /api/rt1/visitor/text HTTP/1.1\r\nHost: localhost:8787\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n".as_slice(),
            b"POST /api/rt1/visitor/text HTTP/1.1\r\nHost: localhost:8787\r\nTransfer-Encoding: chunked\r\n\r\n".as_slice(),
            b"POST /api/rt1/visitor/text HTTP/1.1\r\nHost: localhost:8787\r\nContent-Length: 99999999\r\n\r\n".as_slice(),
        ] {
            assert!(parse_request_header(header).is_err());
        }
    }
}
