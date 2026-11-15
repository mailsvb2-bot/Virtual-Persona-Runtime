//! Bounded HTTP/1.1 response relay for the loopback Owner Lab socket owner.
use super::{
    MAX_HEADER_BYTES, RESPONSE_DEADLINE, RT1_READ_POLL, UPSTREAM_CONNECT_TIMEOUT,
    UPSTREAM_WRITE_TIMEOUT,
};
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::time::Instant;

const MAX_RESPONSE_BODY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResponseFraming {
    ContentLength(usize),
    Chunked,
    CloseDelimited,
}

pub(super) fn relay_ordinary_response(
    upstream_address: SocketAddr,
    request: &[u8],
    client: &mut TcpStream,
) -> io::Result<u16> {
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

    let (status, framing) = parse_response_framing(&header)?;
    let client_header = response_header_with_close(&header)?;
    client.write_all(&client_header)?;
    match framing {
        ResponseFraming::ContentLength(length) => {
            copy_exact_response_body(&mut upstream, client, length, deadline)
        }
        ResponseFraming::Chunked => relay_chunked_response_body(&mut upstream, client, deadline),
        ResponseFraming::CloseDelimited => {
            relay_close_delimited_response_body(&mut upstream, client, deadline)
        }
    }?;
    Ok(status)
}

fn response_header_with_close(header: &[u8]) -> io::Result<Vec<u8>> {
    let text = std::str::from_utf8(header)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP response header"))?;
    if !text.ends_with("\r\n\r\n") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "incomplete HTTP response headers",
        ));
    }
    let mut lines = text[..text.len() - 4].split("\r\n");
    let status = lines.next().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing HTTP response status")
    })?;
    let mut normalized = Vec::with_capacity(header.len() + 24);
    normalized.extend_from_slice(status.as_bytes());
    normalized.extend_from_slice(b"\r\n");
    for line in lines {
        let (name, _) = line.split_once(':').ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP response header")
        })?;
        if name.eq_ignore_ascii_case("connection")
            || name.eq_ignore_ascii_case("keep-alive")
            || name.eq_ignore_ascii_case("proxy-connection")
        {
            continue;
        }
        normalized.extend_from_slice(line.as_bytes());
        normalized.extend_from_slice(b"\r\n");
    }
    normalized.extend_from_slice(b"Connection: close\r\n\r\n");
    Ok(normalized)
}

fn parse_response_framing(header: &[u8]) -> io::Result<(u16, ResponseFraming)> {
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
    let status = lines.next().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing HTTP response status")
    })?;
    let mut status_parts = status.split_whitespace();
    if status_parts.next() != Some("HTTP/1.1") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported HTTP response status",
        ));
    }
    let status_code = status_parts.next().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing HTTP response status code")
    })?;
    if status_code.len() != 3 || !status_code.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported HTTP response status",
        ));
    }
    let status_code = status_code.parse::<u16>().map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP response status code")
    })?;

    let mut content_length = None;
    let mut chunked = false;
    for line in lines {
        let (name, value) = line.split_once(':').ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP response header")
        })?;
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
            content_length = Some(value.trim().parse::<usize>().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid HTTP response content length",
                )
            })?);
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
        return Ok((status_code, ResponseFraming::Chunked));
    }
    if let Some(length) = content_length {
        if length > MAX_RESPONSE_BODY_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ordinary HTTP response body too large",
            ));
        }
        return Ok((status_code, ResponseFraming::ContentLength(length)));
    }
    Ok((status_code, ResponseFraming::CloseDelimited))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn ordinary_response_requires_unambiguous_bounded_framing() {
        let content_length =
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: keep-alive\r\n\r\n";
        assert_eq!(
            parse_response_framing(content_length).unwrap().1,
            ResponseFraming::ContentLength(2)
        );
        assert_eq!(
            parse_response_framing(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .unwrap()
                .1,
            ResponseFraming::Chunked
        );
        assert_eq!(
            parse_response_framing(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                .unwrap()
                .1,
            ResponseFraming::CloseDelimited
        );
        for invalid in [
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nTransfer-Encoding: chunked\r\n\r\n"
                .as_slice(),
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
        let mut client =
            std::net::TcpStream::connect(client_listener.local_addr().unwrap()).unwrap();
        let (mut relay_socket, _) = client_listener.accept().unwrap();
        let started = Instant::now();
        relay_ordinary_response(
            upstream_address,
            b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            &mut relay_socket,
        )
        .unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));

        let expected = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\nWiki\r\n5\r\npedia\r\n0\r\nX-Receipt: test\r\n\r\n";
        let mut actual = vec![0_u8; expected.len()];
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
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
            parse_response_framing(&response[..header_end + 4]).unwrap().1,
            ResponseFraming::ContentLength(6)
        );
        assert!(response.ends_with(b"app.js"));
        let response_header = std::str::from_utf8(&response[..header_end + 4]).unwrap();
        assert!(response_header.contains("Connection: close\r\n"));
        assert!(!response_header.contains("Connection: keep-alive"));
        assert!(!response_header.contains("Keep-Alive:"));
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
        upstream.set_read_timeout(Some(RT1_READ_POLL)).unwrap();

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

        let expected = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: keep-alive\r\n\r\nok";
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
