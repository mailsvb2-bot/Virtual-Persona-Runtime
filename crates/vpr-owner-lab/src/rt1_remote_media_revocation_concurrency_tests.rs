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
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        for (status, audio_stopped, video_stopped) in [
            ("202 Accepted", true, false),
            ("200 OK", true, true),
        ] {
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
            let body = serde_json::json!({
                "operation_id": "partial-av-stop",
                "generation": 11,
                "session_id": "sender-11",
                "audio_stopped": audio_stopped,
                "video_stopped": video_stopped,
            })
            .to_string();
            write!(
                socket,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    let client =
        RemoteOperationClient::new(&endpoint, "worker-secret", Duration::from_secs(3)).unwrap();
    let remote = Rt1RemoteMediaRevocation::new(client, "partial-av-stop".into(), 11).unwrap();
    remote.record_created_session("sender-11").unwrap();
    assert_eq!(
        remote.revoke_confirmed().unwrap_err(),
        "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED"
    );
    assert!(remote.is_revoked());
    assert!(!remote.is_stop_confirmed());
    remote.revoke_confirmed().unwrap();
    assert!(remote.is_stop_confirmed());
    worker.join().unwrap();
}
