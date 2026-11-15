use super::read_request_until;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn request_header_and_body_share_one_deadline_against_slow_drip() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (mut server, _) = listener.accept().unwrap();
    let sender = thread::spawn(move || {
        client.write_all(b"P").unwrap();
        thread::sleep(Duration::from_millis(80));
        let remainder = format!(
            "OST {} HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\n\r\n{{",
            concat!("/api", "/rt1/visitor/text")
        );
        let _ = client.write_all(remainder.as_bytes());
        thread::sleep(Duration::from_millis(120));
        let _ = client.write_all(b"}");
    });
    let started = Instant::now();
    let error = read_request_until(&mut server, started + Duration::from_millis(150)).unwrap_err();
    assert!(matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ));
    assert!(started.elapsed() < Duration::from_millis(500));
    sender.join().unwrap();
}

#[test]
fn slow_header_bytes_cannot_reset_the_total_request_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (mut server, _) = listener.accept().unwrap();
    let sender = thread::spawn(move || {
        let request = format!(
            "POST {} HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n",
            concat!("/api", "/rt1/visitor/text")
        );
        for byte in request.as_bytes() {
            if client.write_all(&[*byte]).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
    });
    let started = Instant::now();
    let error = read_request_until(&mut server, started + Duration::from_millis(120)).unwrap_err();
    assert!(matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ));
    assert!(started.elapsed() < Duration::from_millis(500));
    sender.join().unwrap();
}
