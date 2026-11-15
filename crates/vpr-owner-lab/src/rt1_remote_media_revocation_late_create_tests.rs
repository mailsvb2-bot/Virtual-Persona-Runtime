use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;
use vpr_integration::{CancellationProbe, RealtimeAvatarPort};
use vpr_provider_local_open_source::OperationWorkerAvatar;

fn read_request_headers(socket: &mut TcpStream) -> Vec<u8> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 2048];
    loop {
        let read = socket.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|part| part == b"\\r\\n\\r\\n") {
            break;
        }
    }
    request
}

fn write_json_response(socket: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\\r\\nContent-Type: application/json\\r\\nContent-Length: {}\\r\\nConnection: close\\r\\n\\r\\n{body}",
        body.len()
    );
    socket.write_all(response.as_bytes()).unwrap();
}

fn spawn_late_create_worker(listener: TcpListener) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let responses = [
            (
                "GET /v1/avatar/readiness ",
                "200 OK",
                r#"{"ready":true,"persona_video_ready":true}"#,
            ),
            (
                "PUT /v1/avatar/operations/late-create ",
                "200 OK",
                r#"{"operation_id":"late-create","session_id":"media-late"}"#,
            ),
            (
                "DELETE /v1/avatar/operations/late-create ",
                "200 OK",
                r#"{"operation_id":"late-create","generation":1,"session_id":"media-late","audio_stopped":true,"video_stopped":true}"#,
            ),
        ];
        for (expected, status, body) in responses {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let request = read_request_headers(&mut socket);
            assert!(request.starts_with(expected.as_bytes()));
            write_json_response(&mut socket, status, body);
        }

        listener.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_millis(200);
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((mut socket, _)) => {
                    let mut request = [0_u8; 1024];
                    let read = socket.read(&mut request).unwrap_or(0);
                    panic!(
                        "late CREATE must be compensated before offer/answer; unexpected request: {}",
                        String::from_utf8_lossy(&request[..read])
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("accept unexpected worker request: {error}"),
            }
        }
    })
}

fn build_late_create_avatar(
    client: RemoteOperationClient,
    remote: Arc<Rt1RemoteMediaRevocation>,
) -> OperationWorkerAvatar {
    let create_owner = Arc::clone(&remote);
    let stop_owner = Arc::clone(&remote);
    OperationWorkerAvatar::with_remote_owner(
        client,
        "late-create".to_owned(),
        1,
        move |session_id| {
            create_owner.request_revoke();
            create_owner.record_created_session(session_id)
        },
        move || {
            stop_owner.request_revoke();
            if stop_owner.is_stop_confirmed() {
                Ok(())
            } else {
                Err("RT1_REMOTE_MEDIA_STOP_UNCONFIRMED".to_owned())
            }
        },
    )
    .unwrap()
}

#[test]
fn late_worker_create_id_is_compensated_by_the_registered_owner_before_offer() {
    struct ActiveTurn;
    impl CancellationProbe for ActiveTurn {
        fn is_cancelled(&self) -> bool {
            false
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let worker = spawn_late_create_worker(listener);
    let client =
        RemoteOperationClient::new(&endpoint, "worker-secret", Duration::from_secs(3)).unwrap();
    let remote = Arc::new(
        Rt1RemoteMediaRevocation::new(client.clone(), "late-create".into(), 1).unwrap(),
    );
    let (notify_tx, notify_rx) = std::sync::mpsc::channel();
    remote.install_revoke_notifier(move || {
        let _ = notify_tx.send(());
    });
    let cleanup_remote = Arc::clone(&remote);
    let cleanup = std::thread::spawn(move || {
        notify_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("owner revoke wakes the existing cleanup reconciler");
        cleanup_remote.revoke_confirmed().unwrap();
    });
    let avatar = build_late_create_avatar(client, Arc::clone(&remote));

    assert!(avatar.create_session(&ActiveTurn).is_err());
    cleanup.join().unwrap();
    assert!(remote.is_revoked());
    assert!(remote.is_stop_confirmed());
    worker.join().unwrap();
}

#[test]
fn late_create_identity_binding_does_not_wait_for_stop_network_io() {
    use std::sync::mpsc;
    use std::time::Instant;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (accepted_tx, accepted_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let request = read_request_headers(&mut socket);
        assert!(request.starts_with(b"DELETE /v1/avatar/operations/late-lock "));
        accepted_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let body = r#"{"operation_id":"late-lock","generation":1,"session_id":"late-session","audio_stopped":true,"video_stopped":true}"#;
        write_json_response(&mut socket, "200 OK", body);
    });
    let client =
        RemoteOperationClient::new(&endpoint, "worker-secret", Duration::from_secs(3)).unwrap();
    let remote = Arc::new(Rt1RemoteMediaRevocation::new(client, "late-lock".into(), 1).unwrap());
    remote.request_revoke();
    let stop_remote = Arc::clone(&remote);
    let stop = std::thread::spawn(move || stop_remote.revoke_confirmed());
    accepted_rx.recv_timeout(Duration::from_secs(3)).unwrap();

    let notifications = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let notified = Arc::clone(&notifications);
    remote.install_revoke_notifier(move || {
        notified.fetch_add(1, Ordering::SeqCst);
    });
    let began = Instant::now();
    assert_eq!(
        remote.record_created_session("late-session").unwrap_err(),
        "RT1_VISITOR_REVOKED"
    );
    assert!(
        began.elapsed() < Duration::from_millis(250),
        "late CREATE identity binding waited for the in-flight STOP network request"
    );
    assert_eq!(notifications.load(Ordering::SeqCst), 2);

    release_tx.send(()).unwrap();
    stop.join().unwrap().unwrap();
    assert!(remote.is_stop_confirmed());
    worker.join().unwrap();
}
