//! Public localhost TCP owner for the existing `tiny_http` application.
//! RT1 browser responses keep their actual socket registered with the same
//! runtime session that revokes provider egress; no second HTTP router exists.
use super::{AppState, HTTP_WORKERS};
use std::io::{self, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use vpr_runtime::CancellableTcpWriter;

const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
const MAX_RESPONSE_BODY_BYTES: usize = 16 * 1024 * 1024;
const RESPONSE_DEADLINE: Duration = Duration::from_secs(90);
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(10);
const UPSTREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const UPSTREAM_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const RT1_READ_POLL: Duration = Duration::from_millis(100);
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
        thread::spawn(move || loop {
            let next = receiver.lock().map(|queue| queue.recv());
            let Ok(Ok(stream)) = next else {
                break;
            };
            proxy_connection(stream, upstream, &state);
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

fn proxy_connection(mut stream: TcpStream, upstream: SocketAddr, state: &Arc<AppState>) {
    let _ = stream.set_read_timeout(Some(REQUEST_READ_TIMEOUT));
    let Ok((raw_request, method, path)) = read_request(&mut stream) else {
        write_error(&mut stream, 400, "BAD_REQUEST");
        return;
    };
    let rt1_post = method == "POST"
        && path
            .split('?')
            .next()
            .is_some_and(|value| value.starts_with("/api/rt1/"));
    let request_id = rt1_post.then(|| {
        format!(
            "rt1-{}",
            NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
        )
    });
    let Ok(forwarded) = rewrite_request(&raw_request, request_id.as_deref()) else {
        write_error(&mut stream, 400, "BAD_REQUEST");
        return;
    };

    if let Some(id) = request_id.as_deref() {
        let Ok((mut writer, cancellation)) =
            CancellableTcpWriter::new(stream, RESPONSE_DEADLINE)
        else {
            return;
        };
        if state
            .rt1
            .register_transport_writer(id.to_owned(), cancellation.clone())
            .is_err()
        {
            let _ = writer.write_all_bounded(&error_response(503, "RT1_RESPONSE_WRITER_UNAVAILABLE"));
            return;
        }
        let delivered = relay_rt1_response(
            upstream,
            &forwarded,
            &mut writer,
            &cancellation,
        )
        .is_ok();
        state.rt1.finish_transport_delivery(id, delivered);
    } else {
        let _ = relay_ordinary_response(upstream, &forwarded, &mut stream);
    }
}

fn read_request(
    stream: &mut TcpStream,
) -> io::Result<(Vec<u8>, String, String)> {
    let mut reader = BufReader::new(stream);
    let mut header = Vec::with_capacity(1024);
    let mut byte = [0_u8; 1];
    loop {
        if header.len() >= MAX_HEADER_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "HTTP headers too large"));
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
        return Err(io::Error::new(io::ErrorKind::InvalidData, "incomplete HTTP headers"));
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
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP request line"));
    }
    let mut content_length = None;
    let mut host_count = 0;
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP header"))?;
        if name.trim().is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "empty HTTP header name"));
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "duplicate content length"));
            }
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid content length"))?,
            );
        }
        if name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("expect")
            || name.eq_ignore_ascii_case("upgrade")
        {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "unsupported HTTP framing"));
        }
        if name.eq_ignore_ascii_case("host") {
            host_count += 1;
        }
    }
    if host_count != 1 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "one Host header required"));
    }
    let body_length = content_length.unwrap_or(0);
    if body_length > MAX_BODY_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "HTTP body too large"));
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
    let deadline = Instant::now() + RESPONSE_DEADLINE;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        if cancellation.is_revoked() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "RT1 response revoked"));
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "HTTP response deadline exceeded"));
        }
        match upstream.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => writer.write_all_bounded(&buffer[..count])?,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResponseFraming {
    ContentLength(usize),
    Chunked,
    CloseDelimited,
}

fn relay_ordinary_response(
    upstream_address: SocketAddr,
    request: &[u8],
    client: &mut TcpStream,
) -> io::Result<()> {
    let mut upstream = TcpStream::connect_timeout(&upstream_address, UPSTREAM_CONNECT_TIMEOUT)?;
    upstream.set_write_timeout(Some(UPSTREAM_WRITE_TIMEOUT))?;
    upstream.set_read_timeout(Some(RT1_READ_POLL))?;
    client.set_write_timeout(Some(RESPONSE_DEADLINE))?;
    upstream.write_all(request)?;
    upstream.shutdown(Shutdown::Write)?;

    let deadline = Instant::now() + RESPONSE_DEADLINE;
    let mut header = Vec::with_capacity(1024);
    let mut byte = [0_u8; 1];
    loop {
        read_exact_until_deadline(&mut upstream, &mut byte, deadline)?;
        header.push(byte[0]);
        if header.len() > MAX_HEADER_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ordinary HTTP response headers too large",
            ));
        }
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
    }

    let framing = parse_response_framing(&header)?;
    client.write_all(&header)?;
    match framing {
        ResponseFraming::ContentLength(length) => {
            copy_exact_response_body(&mut upstream, client, length, deadline)
        }
        ResponseFraming::Chunked => relay_chunked_response_body(&mut upstream, client, deadline),
        ResponseFraming::CloseDelimited => {
            relay_close_delimited_response_body(&mut upstream, client, deadline)
        }
    }
}

fn parse_response_framing(header: &[u8]) -> io::Result<ResponseFraming> {
    let text = std::str::from_utf8(header)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP response header"))?;
    if !text.ends_with("\r\n\r\n") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "incomplete HTTP response headers",
        ));
    }
    let mut lines = text[..text.len() - 4]
        .split("\r\n")
        .filter(|line| !line.is_empty());
    let status = lines
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP response status"))?;
    let mut status_parts = status.split_whitespace();
    if status_parts.next() != Some("HTTP/1.1")
        || status_parts
            .next()
            .is_none_or(|code| code.len() != 3 || !code.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported HTTP response status",
        ));
    }

    let mut content_length = None;
    let mut chunked = false;
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP response header"))?;
        if name.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "empty HTTP response header name",
            ));
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate HTTP response content length",
                ));
            }
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP response content length"))?,
            );
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            if chunked || !value.trim().eq_ignore_ascii_case("chunked") {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unsupported HTTP response transfer encoding",
                ));
            }
            chunked = true;
        }
    }
    if content_length.is_some() && chunked {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ambiguous HTTP response body framing",
        ));
    }
    if chunked {
        return Ok(ResponseFraming::Chunked);
    }
    if let Some(length) = content_length {
        if length > MAX_RESPONSE_BODY_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ordinary HTTP response body too large",
            ));
        }
        return Ok(ResponseFraming::ContentLength(length));
    }
    Ok(ResponseFraming::CloseDelimited)
}

fn read_exact_until_deadline(
    stream: &mut TcpStream,
    output: &mut [u8],
    deadline: Instant,
) -> io::Result<()> {
    let mut offset = 0;
    while offset < output.len() {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "ordinary HTTP response deadline exceeded",
            ));
        }
        match stream.read(&mut output[offset..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "ordinary HTTP response truncated",
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

fn copy_exact_response_body(
    upstream: &mut TcpStream,
    client: &mut TcpStream,
    length: usize,
    deadline: Instant,
) -> io::Result<()> {
    let mut remaining = length;
    let mut buffer = [0_u8; 16 * 1024];
    while remaining > 0 {
        let count = remaining.min(buffer.len());
        read_exact_until_deadline(upstream, &mut buffer[..count], deadline)?;
        client.write_all(&buffer[..count])?;
        remaining -= count;
    }
    Ok(())
}

fn read_chunk_line(upstream: &mut TcpStream, deadline: Instant) -> io::Result<Vec<u8>> {
    let mut line = Vec::with_capacity(32);
    let mut byte = [0_u8; 1];
    loop {
        read_exact_until_deadline(upstream, &mut byte, deadline)?;
        line.push(byte[0]);
        if line.ends_with(b"\r\n") {
            return Ok(line);
        }
        if line.len() > 8 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP chunk line too large",
            ));
        }
    }
}

fn relay_chunked_response_body(
    upstream: &mut TcpStream,
    client: &mut TcpStream,
    deadline: Instant,
) -> io::Result<()> {
    let mut total = 0_usize;
    loop {
        let line = read_chunk_line(upstream, deadline)?;
        let size_line = std::str::from_utf8(&line[..line.len() - 2])
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP chunk size"))?;
        let size_text = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP chunk size"))?;
        if size > MAX_RESPONSE_BODY_BYTES.saturating_sub(total) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ordinary HTTP response body too large",
            ));
        }
        client.write_all(&line)?;
        if size == 0 {
            let mut trailer_bytes = 0;
            loop {
                let trailer = read_chunk_line(upstream, deadline)?;
                trailer_bytes += trailer.len();
                if trailer_bytes > MAX_HEADER_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "HTTP response trailers too large",
                    ));
                }
                client.write_all(&trailer)?;
                if trailer == b"\r\n" {
                    return Ok(());
                }
            }
        }
        copy_exact_response_body(upstream, client, size, deadline)?;
        total += size;
        let mut terminator = [0_u8; 2];
        read_exact_until_deadline(upstream, &mut terminator, deadline)?;
        if terminator != *b"\r\n" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid HTTP chunk terminator",
            ));
        }
        client.write_all(&terminator)?;
    }
}

fn relay_close_delimited_response_body(
    upstream: &mut TcpStream,
    client: &mut TcpStream,
    deadline: Instant,
) -> io::Result<()> {
    let mut total = 0_usize;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "close-delimited HTTP response deadline exceeded",
            ));
        }
        match upstream.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => {
                total = total.saturating_add(count);
                if total > MAX_RESPONSE_BODY_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "ordinary HTTP response body too large",
                    ));
                }
                client.write_all(&buffer[..count])?;
            }
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
    use super::{
        parse_request_header, parse_response_framing, relay_ordinary_response, rewrite_request,
        ResponseFraming,
    };
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn proxy_injects_unspoofable_transport_identity_and_closes_upstream() {
        let raw = b"POST /api/rt1/visitor/text HTTP/1.1\r\nHost: 127.0.0.1:8787\r\nOrigin: http://127.0.0.1:8787\r\nConnection: keep-alive\r\nX-VPR-RT1-Transport-Id: spoof\r\nContent-Length: 2\r\n\r\n{}";
        let forwarded = rewrite_request(raw, Some("rt1-9")).unwrap();
        let header_end = forwarded.windows(4).position(|window| window == b"\r\n\r\n").unwrap();
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
    fn proxy_rejects_ambiguous_or_unbounded_request_framing() {
        for header in [
            b"POST /api/rt1/visitor/text HTTP/1.1\r\nHost: localhost:8787\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n".as_slice(),
            b"POST /api/rt1/visitor/text HTTP/1.1\r\nHost: localhost:8787\r\nTransfer-Encoding: chunked\r\n\r\n".as_slice(),
            b"POST /api/rt1/visitor/text HTTP/1.1\r\nHost: localhost:8787\r\nContent-Length: 99999999\r\n\r\n".as_slice(),
        ] {
            assert!(parse_request_header(header).is_err());
        }
    }

    #[test]
    fn ordinary_response_requires_unambiguous_bounded_framing() {
        let content_length =
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: keep-alive\r\n\r\n";
        assert_eq!(
            parse_response_framing(content_length).unwrap(),
            ResponseFraming::ContentLength(2)
        );
        assert_eq!(
            parse_response_framing(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .unwrap(),
            ResponseFraming::Chunked
        );
        assert_eq!(
            parse_response_framing(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n").unwrap(),
            ResponseFraming::CloseDelimited
        );
        for invalid in [
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nTransfer-Encoding: chunked\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 999999999\r\n\r\n".as_slice(),
        ] {
            assert!(parse_response_framing(invalid).is_err());
        }
    }

    #[test]
    fn ordinary_relay_returns_at_chunk_terminator_while_upstream_keeps_connection_open() {
        let upstream_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_address = upstream_listener.local_addr().unwrap();
        let (release_sender, release_receiver) = mpsc::channel();
        let upstream = thread::spawn(move || {
            let (mut socket, _) = upstream_listener.accept().unwrap();
            let mut request_header = Vec::new();
            let mut byte = [0_u8; 1];
            loop {
                socket.read_exact(&mut byte).unwrap();
                request_header.push(byte[0]);
                if request_header.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n4\r\nWiki\r\n5\r\npedia\r\n0\r\nX-Receipt: test\r\n\r\n",
                )
                .unwrap();
            release_receiver
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
        });

        let client_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = std::net::TcpStream::connect(client_listener.local_addr().unwrap()).unwrap();
        let (mut relay_socket, _) = client_listener.accept().unwrap();
        let started = Instant::now();
        relay_ordinary_response(
            upstream_address,
            b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            &mut relay_socket,
        )
        .unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));

        let expected = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n4\r\nWiki\r\n5\r\npedia\r\n0\r\nX-Receipt: test\r\n\r\n";
        let mut actual = vec![0_u8; expected.len()];
        client.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        client.read_exact(&mut actual).unwrap();
        assert_eq!(actual, expected);
        release_sender.send(()).unwrap();
        upstream.join().unwrap();
    }

    #[test]
    fn ordinary_relay_handles_the_actual_tiny_http_cursor_response() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let upstream_address = server.server_addr().to_ip().unwrap();
        let upstream = thread::spawn(move || {
            let request = server.recv().unwrap();
            request
                .respond(tiny_http::Response::from_data(b"app.js".to_vec()))
                .unwrap();
        });

        let client_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client =
            std::net::TcpStream::connect(client_listener.local_addr().unwrap()).unwrap();
        let (mut relay_socket, _) = client_listener.accept().unwrap();
        relay_ordinary_response(
            upstream_address,
            b"GET /app.js HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            &mut relay_socket,
        )
        .unwrap();
        drop(relay_socket);

        let mut response = Vec::new();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        client.read_to_end(&mut response).unwrap();
        let header_end = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("tiny_http response header");
        assert_eq!(
            parse_response_framing(&response[..header_end + 4]).unwrap(),
            ResponseFraming::ContentLength(6)
        );
        assert!(response.ends_with(b"app.js"));
        upstream.join().unwrap();
    }

    #[test]
    fn close_delimited_relay_has_an_absolute_deadline_when_upstream_stays_open() {
        let upstream_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_address = upstream_listener.local_addr().unwrap();
        let (release_sender, release_receiver) = mpsc::channel();
        let upstream_server = thread::spawn(move || {
            let (mut socket, _) = upstream_listener.accept().unwrap();
            socket.write_all(b"partial").unwrap();
            release_receiver
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
        });

        let client_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client =
            std::net::TcpStream::connect(client_listener.local_addr().unwrap()).unwrap();
        let (mut relay_socket, _) = client_listener.accept().unwrap();
        let mut upstream = std::net::TcpStream::connect(upstream_address).unwrap();
        upstream
            .set_read_timeout(Some(RT1_READ_POLL))
            .unwrap();

        let started = Instant::now();
        let error = relay_close_delimited_response_body(
            &mut upstream,
            &mut relay_socket,
            started + Duration::from_millis(250),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
        let mut partial = [0_u8; 7];
        client.read_exact(&mut partial).unwrap();
        assert_eq!(&partial, b"partial");

        release_sender.send(()).unwrap();
        upstream_server.join().unwrap();
    }

    #[test]
    fn ordinary_relay_returns_at_content_length_while_upstream_keeps_connection_open() {
        let upstream_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_address = upstream_listener.local_addr().unwrap();
        let (release_sender, release_receiver) = mpsc::channel();
        let upstream = thread::spawn(move || {
            let (mut socket, _) = upstream_listener.accept().unwrap();
            let mut request_header = Vec::new();
            let mut byte = [0_u8; 1];
            loop {
                socket.read_exact(&mut byte).unwrap();
                request_header.push(byte[0]);
                if request_header.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: keep-alive\r\n\r\nok",
                )
                .unwrap();
            release_receiver
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
        });

        let client_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client_address = client_listener.local_addr().unwrap();
        let mut client = std::net::TcpStream::connect(client_address).unwrap();
        let (mut relay_socket, _) = client_listener.accept().unwrap();
        let started = Instant::now();
        relay_ordinary_response(
            upstream_address,
            b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            &mut relay_socket,
        )
        .unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));

        let expected =
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: keep-alive\r\n\r\nok";
        let mut actual = vec![0_u8; expected.len()];
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        client.read_exact(&mut actual).unwrap();
        assert_eq!(actual, expected);
        release_sender.send(()).unwrap();
        upstream.join().unwrap();
    }
}
