//! Public localhost TCP owner for the existing tiny_http application.
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
const RESPONSE_DEADLINE: Duration = Duration::from_secs(90);
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(10);
const UPSTREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const UPSTREAM_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const UPSTREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const RT1_READ_POLL: Duration = Duration::from_millis(100);
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

pub(super) fn serve(
    listener: TcpListener,
    upstream: SocketAddr,
    state: Arc<AppState>,
) -> io::Result<()> {
    let (sender, receiver) = mpsc::sync_channel::<TcpStream>(64);
    let receiver = Arc::new(Mutex::new(receiver));
    for _ in 0..HTTP_WORKERS {
        let receiver = Arc::clone(&receiver);
        let state = Arc::clone(&state);
        thread::spawn(move || loop {
            let next = receiver.lock().map(|queue| queue.recv());
            let stream = match next {
                Ok(Ok(stream)) => stream,
                _ => break,
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
    let (raw_request, method, path) = match read_request(&mut stream) {
        Ok(request) => request,
        Err(_) => {
            write_error(&mut stream, 400, "BAD_REQUEST");
            return;
        }
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
    let forwarded = match rewrite_request(&raw_request, request_id.as_deref()) {
        Ok(request) => request,
        Err(_) => {
            write_error(&mut stream, 400, "BAD_REQUEST");
            return;
        }
    };

    if let Some(id) = request_id.as_deref() {
        let (mut writer, cancellation) =
            match CancellableTcpWriter::new(stream, RESPONSE_DEADLINE) {
                Ok(writer) => writer,
                Err(_) => return,
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
        let name = line.split_once(':').map(|(name, _)| name.trim()).unwrap_or("");
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
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        }
    }
}

fn relay_ordinary_response(
    upstream_address: SocketAddr,
    request: &[u8],
    client: &mut TcpStream,
) -> io::Result<()> {
    let mut upstream = TcpStream::connect_timeout(&upstream_address, UPSTREAM_CONNECT_TIMEOUT)?;
    upstream.set_write_timeout(Some(UPSTREAM_WRITE_TIMEOUT))?;
    upstream.write_all(request)?;
    upstream.shutdown(Shutdown::Write)?;
    io::copy(&mut upstream, client)?;
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
        let header = format!(
            "POST /api/rt1/visitor/voice HTTP/1.1\r\nHost: localhost:8787\r\nContent-Length: 3844096\r\n\r\n"
        );
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
}
