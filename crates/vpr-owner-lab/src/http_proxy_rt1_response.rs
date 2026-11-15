//! Revocation-aware framing for RT1 responses sent through the public socket owner.
use std::io::{self, Read};
use std::net::TcpStream;
use std::time::Instant;
use vpr_runtime::TcpWriteRevocation;

use super::{
    MAX_RESPONSE_BODY_BYTES, ResponseFraming, parse_response_framing, response_header_with_close,
};

pub(crate) fn relay_revocable_response_body(
    upstream: &mut TcpStream,
    header: &[u8],
    deadline: Instant,
    cancellation: &TcpWriteRevocation,
    mut write: impl FnMut(&[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let (_, framing) = parse_response_framing(header)?;
    let normalized_header = response_header_with_close(header)?;
    write(&normalized_header)?;
    match framing {
        ResponseFraming::ContentLength(length) => {
            copy_exact_body(upstream, length, deadline, cancellation, &mut write)
        }
        ResponseFraming::Chunked => {
            relay_chunked_body(upstream, deadline, cancellation, &mut write)
        }
        ResponseFraming::CloseDelimited => {
            relay_close_delimited_body(upstream, deadline, cancellation, &mut write)
        }
    }
}

fn ensure_live(deadline: Instant, cancellation: &TcpWriteRevocation) -> io::Result<()> {
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
    Ok(())
}

fn read_exact_revocable(
    upstream: &mut TcpStream,
    output: &mut [u8],
    deadline: Instant,
    cancellation: &TcpWriteRevocation,
) -> io::Result<()> {
    let mut offset = 0;
    while offset < output.len() {
        ensure_live(deadline, cancellation)?;
        match upstream.read(&mut output[offset..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "RT1 response framing truncated",
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

fn copy_exact_body(
    upstream: &mut TcpStream,
    length: usize,
    deadline: Instant,
    cancellation: &TcpWriteRevocation,
    write: &mut impl FnMut(&[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let mut remaining = length;
    let mut buffer = [0_u8; 16 * 1024];
    while remaining > 0 {
        ensure_live(deadline, cancellation)?;
        let limit = remaining.min(buffer.len());
        match upstream.read(&mut buffer[..limit]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "RT1 response body shorter than Content-Length",
                ));
            }
            Ok(count) => {
                write(&buffer[..count])?;
                remaining -= count;
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
    Ok(())
}

fn read_chunk_line(
    upstream: &mut TcpStream,
    deadline: Instant,
    cancellation: &TcpWriteRevocation,
) -> io::Result<Vec<u8>> {
    let mut line = Vec::with_capacity(32);
    let mut byte = [0_u8; 1];
    loop {
        read_exact_revocable(upstream, &mut byte, deadline, cancellation)?;
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

fn relay_chunked_body(
    upstream: &mut TcpStream,
    deadline: Instant,
    cancellation: &TcpWriteRevocation,
    write: &mut impl FnMut(&[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let mut total = 0_usize;
    loop {
        let line = read_chunk_line(upstream, deadline, cancellation)?;
        let size_line = std::str::from_utf8(&line[..line.len() - 2])
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP chunk size"))?;
        let size_text = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP chunk size"))?;
        if size > MAX_RESPONSE_BODY_BYTES.saturating_sub(total) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "RT1 response body too large",
            ));
        }
        write(&line)?;
        if size == 0 {
            let mut trailer_bytes = 0;
            loop {
                let trailer = read_chunk_line(upstream, deadline, cancellation)?;
                trailer_bytes += trailer.len();
                if trailer_bytes > super::super::MAX_HEADER_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "HTTP response trailers too large",
                    ));
                }
                write(&trailer)?;
                if trailer == b"\r\n" {
                    return Ok(());
                }
            }
        }
        copy_exact_body(upstream, size, deadline, cancellation, write)?;
        total += size;
        let mut terminator = [0_u8; 2];
        read_exact_revocable(upstream, &mut terminator, deadline, cancellation)?;
        if terminator != *b"\r\n" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid HTTP chunk terminator",
            ));
        }
        write(&terminator)?;
    }
}

fn relay_close_delimited_body(
    upstream: &mut TcpStream,
    deadline: Instant,
    cancellation: &TcpWriteRevocation,
    write: &mut impl FnMut(&[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let mut total = 0_usize;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        ensure_live(deadline, cancellation)?;
        match upstream.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => {
                total = total.saturating_add(count);
                if total > MAX_RESPONSE_BODY_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "RT1 response body too large",
                    ));
                }
                write(&buffer[..count])?;
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
