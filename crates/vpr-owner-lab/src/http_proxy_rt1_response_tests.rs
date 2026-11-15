//! Loopback regressions for the revocable RT1 public HTTP response path.
use super::relay_rt1_response;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};
use vpr_runtime::CancellableTcpWriter;

#[test]
fn rt1_response_relay_returns_at_content_length_while_upstream_keeps_connection_open() {
    let upstream_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let upstream_address = upstream_listener.local_addr().unwrap();
    let (release_sender, release_receiver) = std::sync::mpsc::channel();
    let upstream_thread = thread::spawn(move || {
        let (mut socket, _) = upstream_listener.accept().unwrap();
        let mut request = Vec::new();
        socket.read_to_end(&mut request).unwrap();
        socket
            .write_all(
                b"HTTP/1.1 200 OK\\r\\nContent-Length: 2\\r\\nConnection: keep-alive\\r\\n\\r\\nok",
            )
            .unwrap();
        release_receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
    });

    let public_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut browser = TcpStream::connect(public_listener.local_addr().unwrap()).unwrap();
    let (public_socket, _) = public_listener.accept().unwrap();
    browser.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let (mut writer, cancellation) =
        CancellableTcpWriter::new(public_socket, Duration::from_secs(2)).unwrap();
    let started = Instant::now();
    relay_rt1_response(
        upstream_address,
        b"POST /api/rt1/visitor/text HTTP/1.1\\r\\nHost: localhost\\r\\nContent-Length: 2\\r\\n\\r\\n{}",
        &mut writer,
        &cancellation,
    )
    .unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));

    let expected = b"HTTP/1.1 200 OK\\r\\nContent-Length: 2\\r\\nConnection: close\\r\\n\\r\\nok";
    let mut actual = vec![0_u8; expected.len()];
    browser.read_exact(&mut actual).unwrap();
    assert_eq!(actual, expected);
    drop(writer);
    release_sender.send(()).unwrap();
    upstream_thread.join().unwrap();
}

#[test]
fn rt1_response_relay_rejects_truncated_content_length_for_delivery_compensation() {
    let upstream_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let upstream_address = upstream_listener.local_addr().unwrap();
    let upstream_thread = thread::spawn(move || {
        let (mut socket, _) = upstream_listener.accept().unwrap();
        let mut request = Vec::new();
        socket.read_to_end(&mut request).unwrap();
        socket
            .write_all(
                b"HTTP/1.1 200 OK\\r\\nContent-Length: 12\\r\\nConnection: close\\r\\n\\r\\npartial",
            )
            .unwrap();
        socket.shutdown(std::net::Shutdown::Write).unwrap();
    });

    let public_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut browser = TcpStream::connect(public_listener.local_addr().unwrap()).unwrap();
    let (public_socket, _) = public_listener.accept().unwrap();
    browser.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let (mut writer, cancellation) =
        CancellableTcpWriter::new(public_socket, Duration::from_secs(2)).unwrap();
    let error = relay_rt1_response(
        upstream_address,
        b"POST /api/rt1/visitor/start HTTP/1.1\\r\\nHost: localhost\\r\\nContent-Length: 2\\r\\n\\r\\n{}",
        &mut writer,
        &cancellation,
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    drop(writer);

    let mut received = Vec::new();
    browser.read_to_end(&mut received).unwrap();
    assert!(received.ends_with(b"partial"));
    upstream_thread.join().unwrap();
}
