use super::*;

#[test]
fn concurrent_owner_revokes_share_one_confirmed_remote_stop() {
    use std::sync::Barrier;

    // The worker accepts exactly one STOP. A second HTTP cancellation would
    // block until timeout and fail this test; both owner callbacks must see
    // the same confirmed audio/video receipt through the canonical stop gate.
    let (endpoint, worker) = single_stop_receipt("parallel-revoke", 9, "sender-9");
    let client =
        RemoteOperationClient::new(&endpoint, "worker-secret", Duration::from_secs(2)).unwrap();
    let remote =
        Arc::new(Rt1RemoteMediaRevocation::new(client, "parallel-revoke".into(), 9).unwrap());
    remote.record_created_session("sender-9").unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let mut callbacks = Vec::new();
    for _ in 0..2 {
        let remote = Arc::clone(&remote);
        let barrier = Arc::clone(&barrier);
        callbacks.push(std::thread::spawn(move || {
            barrier.wait();
            remote.revoke_confirmed()
        }));
    }
    barrier.wait();
    for callback in callbacks {
        callback.join().unwrap().unwrap();
    }
    assert!(remote.is_revoked());
    assert!(remote.is_stop_confirmed());
    worker.join().unwrap();
}

#[test]
fn partial_audio_stop_receipt_never_confirms_video_and_retries_same_sender() {
    use std::sync::mpsc;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (pending_tx, pending_rx) = mpsc::channel();
    let (retry_tx, retry_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        for attempt in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 2048];
            loop {
                let read = socket.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
            }
            assert!(request.starts_with(b"DELETE /v1/avatar/operations/partial-av-stop "));
            if attempt == 0 {
                // Audio may already be stopped while video is still live.
                // The worker must return pending, never a partial success receipt.
                let body = r#"{"operation_id":"partial-av-stop","stop_confirmed":false}"#;
                write!(
                    socket,
                    "HTTP/1.1 202 Accepted\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
                pending_tx.send(()).unwrap();
            } else {
                retry_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
                let body = r#"{"operation_id":"partial-av-stop","generation":11,"session_id":"sender-11","audio_stopped":true,"video_stopped":true}"#;
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        }
    });
    let client =
        RemoteOperationClient::new(&endpoint, "worker-secret", Duration::from_secs(3)).unwrap();
    let remote =
        Arc::new(Rt1RemoteMediaRevocation::new(client, "partial-av-stop".into(), 11).unwrap());
    remote.record_created_session("sender-11").unwrap();

    let stopping = Arc::clone(&remote);
    let stop = std::thread::spawn(move || stopping.revoke_confirmed());
    pending_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(remote.is_revoked());
    assert!(!remote.is_stop_confirmed());
    retry_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(
        !remote.is_stop_confirmed(),
        "audio-only completion cannot be reported as dual-track STOP"
    );
    release_tx.send(()).unwrap();
    stop.join().unwrap().unwrap();
    assert!(remote.is_stop_confirmed());
    worker.join().unwrap();
}
