//! A single, bounded TCP write owner that can be interrupted by revocation.
//!
//! The existing `tiny_http::Request::respond` hides its socket and cannot
//! use this writer. The RT1 HTTP boundary must own the connection before it
//! can rely on this primitive; no release claim follows from adding it alone.

use std::io::{self, ErrorKind, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

const MAX_WRITE_CHUNK: usize = 16 * 1024;
const MAX_SYSCALL_WAIT: Duration = Duration::from_millis(100);

struct Connection {
    stream: TcpStream,
    revoked: AtomicBool,
}

/// A cloneable handle that interrupts an in-flight TCP send and fences later sends.
#[derive(Clone)]
pub struct TcpWriteRevocation {
    connection: Arc<Connection>,
}

impl TcpWriteRevocation {
    /// Permanently revoke the connection and wake a pending blocking write.
    ///
    /// This is only a LOCAL transport abort, not proof of browser receipt or
    /// of termination of a remote audio/video sender.
    pub fn revoke(&self) {
        self.connection.revoked.store(true, Ordering::Release);
        let _ = self.connection.stream.shutdown(Shutdown::Both);
    }

    /// Whether this connection has been locally revoked.
    #[must_use]
    pub fn is_revoked(&self) -> bool {
        self.connection.revoked.load(Ordering::Acquire)
    }
}

/// The only writer for a TCP response, with a total wall-clock deadline.
pub struct CancellableTcpWriter {
    connection: Arc<Connection>,
    deadline: Instant,
}

impl CancellableTcpWriter {
    /// Adopt an already-authorized TCP stream and return its revocation handle.
    ///
    /// The caller must perform the publication admission check before writing.
    /// This API does not duplicate or replace publication authorization.
    #[must_use]
    pub fn new(stream: TcpStream, max_duration: Duration) -> io::Result<(Self, TcpWriteRevocation)> {
        // A previously nonblocking socket would otherwise busy-spin on WouldBlock.
        stream.set_nonblocking(false)?;
        let connection = Arc::new(Connection {
            stream,
            revoked: AtomicBool::new(false),
        });
        Ok((
            Self {
                connection: Arc::clone(&connection),
                deadline: Instant::now() + max_duration,
            },
            TcpWriteRevocation { connection },
        ))
    }

    /// Send all bytes in bounded chunks, or abort the TCP connection.
    ///
    /// No success is reported after a local revoke. Once a syscall has
    /// entered the kernel it may have sent a prefix; there is no way to
    /// retract bytes already accepted by the peer.
    ///
    /// # Errors
    /// Returns an I/O error if the socket fails, the deadline expires, or
    /// the session is revoked before the entire buffer can be written.
    pub fn write_all_bounded(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut remaining = bytes;
        while !remaining.is_empty() {
            if self.connection.revoked.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    ErrorKind::Interrupted,
                    "TCP response revoked",
                ));
            }
            let Some(left) = self.deadline.checked_duration_since(Instant::now()) else {
                self.abort();
                return Err(io::Error::new(
                    ErrorKind::TimedOut,
                    "TCP response deadline exceeded",
                ));
            };
            if left.is_zero() {
                self.abort();
                return Err(io::Error::new(
                    ErrorKind::TimedOut,
                    "TCP response deadline exceeded",
                ));
            }
            self.connection
                .stream
                .set_write_timeout(Some(left.min(MAX_SYSCALL_WAIT)))?;
            let count = remaining.len().min(MAX_WRITE_CHUNK);
            let mut socket = &self.connection.stream;
            match socket.write(&remaining[..count]) {
                Ok(0) => {
                    self.abort();
                    return Err(io::Error::new(ErrorKind::WriteZero, "TCP response closed"));
                }
                Ok(written) => remaining = &remaining[written..],
                Err(error)
                    if matches!(
                        error.kind(),
                        ErrorKind::Interrupted | ErrorKind::TimedOut | ErrorKind::WouldBlock
                    ) =>
                {
                    if self.connection.revoked.load(Ordering::Acquire) {
                        return Err(io::Error::new(
                            ErrorKind::Interrupted,
                            "TCP response revoked",
                        ));
                    }
                }
                Err(error) => {
                    self.abort();
                    return Err(error);
                }
            }
        }
        if self.connection.revoked.load(Ordering::Acquire) {
            return Err(io::Error::new(
                ErrorKind::Interrupted,
                "TCP response revoked",
            ));
        }
        Ok(())
    }

    fn abort(&self) {
        self.connection.revoked.store(true, Ordering::Release);
        let _ = self.connection.stream.shutdown(Shutdown::Both);
    }
}

// The response writer owns the lifetime of the socket. Revocation handles
// must not keep a completed or abandoned response connection alive.
impl Drop for CancellableTcpWriter {
    fn drop(&mut self) {
        self.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;
    use std::net::TcpListener;
    use std::thread;

    fn loopback_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        (server, client)
    }

    #[test]
    fn authorized_writer_delivers_bytes() {
        let (server, mut client) = loopback_pair();
        let (mut writer, cancel) = CancellableTcpWriter::new(server, Duration::from_secs(2)).unwrap();
        writer.write_all_bounded(b"persona").unwrap();
        let mut data = [0; 7];
        client.read_exact(&mut data).unwrap();
        assert_eq!(&data, b"persona");
        assert!(!cancel.is_revoked());
    }

    #[test]
    fn revoke_rejects_new_bytes_and_keeps_abort_permanent() {
        let (server, mut client) = loopback_pair();
        let (mut writer, cancel) = CancellableTcpWriter::new(server, Duration::from_secs(2)).unwrap();
        cancel.revoke();
        assert!(cancel.is_revoked());
        assert_eq!(
            writer.write_all_bounded(b"never send").unwrap_err().kind(),
            ErrorKind::Interrupted
        );
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut buf = [0u8; 1];
        assert_eq!(client.read(&mut buf).unwrap(), 0);
    }

    #[test]
    fn expired_writer_aborts_before_network_delivery() {
        let (server, mut client) = loopback_pair();
        let (mut writer, cancel) = CancellableTcpWriter::new(server, Duration::ZERO).unwrap();
        assert_eq!(
            writer.write_all_bounded(b"never send").unwrap_err().kind(),
            ErrorKind::TimedOut
        );
        assert!(cancel.is_revoked());
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut buf = [0u8; 1];
        assert_eq!(client.read(&mut buf).unwrap(), 0);
    }

    #[test]
    fn dropping_writer_closes_socket_even_if_revocation_handle_survives() {
        let (server, mut client) = loopback_pair();
        let (mut writer, cancel) =
            CancellableTcpWriter::new(server, Duration::from_secs(2)).unwrap();
        writer.write_all_bounded(b"done").unwrap();
        drop(writer);
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut data = [0u8; 4];
        client.read_exact(&mut data).unwrap();
        assert_eq!(&data, b"done");
        let mut extra = [0u8; 1];
        assert_eq!(client.read(&mut extra).unwrap(), 0);
        assert!(cancel.is_revoked());
    }

    #[test]
    fn concurrent_revoke_is_observed_by_writer() {
        let (server, _client) = loopback_pair();
        // Adopting an inherited nonblocking descriptor must not busy-spin.
        server.set_nonblocking(true).unwrap();
        let (mut writer, cancel) = CancellableTcpWriter::new(server, Duration::from_secs(3)).unwrap();
        let send = thread::spawn(move || writer.write_all_bounded(&vec![17u8; 32 * 1024 * 1024]));
        thread::sleep(Duration::from_millis(25));
        cancel.revoke();
        assert!(send.join().unwrap().is_err());
        assert!(cancel.is_revoked());
    }
}
