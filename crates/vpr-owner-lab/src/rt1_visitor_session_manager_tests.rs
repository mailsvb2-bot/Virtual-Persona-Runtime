use super::*;

#[test]
fn drained_transports_stop_once_outside_registry_lock() {
    use std::sync::atomic::AtomicUsize;

    let manager = Arc::new(VisitorSessionManager::new(Arc::new(
        Rt1WorkspaceService::new(),
    )));
    let stops = Arc::new(AtomicUsize::new(0));
    let callback_manager = Arc::clone(&manager);
    let callback_stops = Arc::clone(&stops);
    let session = Arc::new(ManagedVisitor {
        egress: Arc::new(VisitorEgressState::default()),
        authority: Rt1VisitorSession::inert_for_revocation_test(),
        revoked: AtomicBool::new(false),
        stopped: AtomicBool::new(false),
        fast_revoke: Arc::new(|| {}),
        stop: Arc::new(move || {
            // A stop callback may re-enter the registry; no mutex is held.
            assert_eq!(callback_manager.active_count(), 0);
            callback_stops.fetch_add(1, Ordering::SeqCst);
        }),
    });
    manager
        .sessions
        .lock()
        .insert("old-publication".into(), Arc::clone(&session));

    let pending = manager.take_for_revocation();
    assert_eq!(manager.active_count(), 0);
    assert!(session.revoked.load(Ordering::Acquire));
    assert_eq!(stops.load(Ordering::SeqCst), 0);
    pending.revoke();
    assert!(session.revoked.load(Ordering::Acquire));
    session.revoke();
    assert_eq!(stops.load(Ordering::SeqCst), 1);
}

#[test]
fn publication_drain_revokes_runtime_permits_before_remote_stop() {
    use std::sync::atomic::AtomicUsize;
    let manager = VisitorSessionManager::new(Arc::new(Rt1WorkspaceService::new()));
    let fast = Arc::new(AtomicUsize::new(0));
    let slow = Arc::new(AtomicUsize::new(0));
    let fast_count = Arc::clone(&fast);
    let slow_count = Arc::clone(&slow);
    manager.sessions.lock().insert(
        "egress".into(),
        Arc::new(ManagedVisitor {
            egress: Arc::new(VisitorEgressState::default()),
            authority: Rt1VisitorSession::inert_for_revocation_test(),
            revoked: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            fast_revoke: Arc::new(move || {
                fast_count.fetch_add(1, Ordering::SeqCst);
            }),
            stop: Arc::new(move || {
                slow_count.fetch_add(1, Ordering::SeqCst);
            }),
        }),
    );
    let pending = manager.take_for_revocation();
    assert_eq!(fast.load(Ordering::SeqCst), 1);
    assert_eq!(slow.load(Ordering::SeqCst), 0);
    assert!(manager.authorize("egress").is_err());
    pending.revoke();
    assert_eq!(slow.load(Ordering::SeqCst), 1);
    manager.revoke_all();
    assert_eq!(fast.load(Ordering::SeqCst), 1);
    assert_eq!(slow.load(Ordering::SeqCst), 1);
}

#[test]
fn revoke_does_not_wait_for_stalled_provider_worker() {
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;
    use std::time::Duration;

    let manager = Arc::new(VisitorSessionManager::new(Arc::new(
        Rt1WorkspaceService::new(),
    )));
    // Simulate an unresponsive provider worker holding its conversation
    // engine mutex. The canonical registry must never acquire that mutex
    // while revoking admission or running the transport stop callback.
    let engine = Arc::new(Mutex::new(()));
    let (worker_locked_tx, worker_locked_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker_engine = Arc::clone(&engine);
    let worker = std::thread::spawn(move || {
        let _held = worker_engine.lock();
        worker_locked_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    });
    worker_locked_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap();

    let stops = Arc::new(AtomicUsize::new(0));
    let stop_count = Arc::clone(&stops);
    manager.sessions.lock().insert(
        "stalled".into(),
        Arc::new(ManagedVisitor {
            egress: Arc::new(VisitorEgressState::default()),
            authority: Rt1VisitorSession::inert_for_revocation_test(),
            revoked: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            fast_revoke: Arc::new(|| {}),
            stop: Arc::new(move || {
                stop_count.fetch_add(1, Ordering::SeqCst);
            }),
        }),
    );

    let (revoked_tx, revoked_rx) = mpsc::channel();
    let revoking_manager = Arc::clone(&manager);
    let revoker = std::thread::spawn(move || {
        revoking_manager.revoke_all();
        revoked_tx.send(()).unwrap();
    });
    // Unstick the fake worker even on failure so the test cannot hang CI.
    let promptly_revoked = revoked_rx.recv_timeout(Duration::from_secs(2)).is_ok();
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    revoker.join().unwrap();
    assert!(
        promptly_revoked,
        "visitor revocation blocked on provider worker"
    );
    assert_eq!(manager.active_count(), 0);
    assert_eq!(stops.load(Ordering::SeqCst), 1);
}

#[test]
fn registry_drain_does_not_stop_sessions_added_after_drain() {
    use std::sync::atomic::AtomicUsize;

    let manager = VisitorSessionManager::new(Arc::new(Rt1WorkspaceService::new()));
    let old_stops = Arc::new(AtomicUsize::new(0));
    let new_stops = Arc::new(AtomicUsize::new(0));
    let old_counter = Arc::clone(&old_stops);
    let new_counter = Arc::clone(&new_stops);
    let old_session = Arc::new(ManagedVisitor {
        egress: Arc::new(VisitorEgressState::default()),
        authority: Rt1VisitorSession::inert_for_revocation_test(),
        revoked: AtomicBool::new(false),
        stopped: AtomicBool::new(false),
        fast_revoke: Arc::new(|| {}),
        stop: Arc::new(move || {
            old_counter.fetch_add(1, Ordering::SeqCst);
        }),
    });
    let new_session = Arc::new(ManagedVisitor {
        egress: Arc::new(VisitorEgressState::default()),
        authority: Rt1VisitorSession::inert_for_revocation_test(),
        revoked: AtomicBool::new(false),
        stopped: AtomicBool::new(false),
        fast_revoke: Arc::new(|| {}),
        stop: Arc::new(move || {
            new_counter.fetch_add(1, Ordering::SeqCst);
        }),
    });
    manager.sessions.lock().insert("old".into(), old_session);
    let pending = manager.take_for_revocation();
    manager.sessions.lock().insert("new".into(), new_session);
    pending.revoke();
    assert_eq!(old_stops.load(Ordering::SeqCst), 1);
    assert_eq!(new_stops.load(Ordering::SeqCst), 0);
    assert_eq!(manager.active_count(), 1);
    manager.revoke_all();
    assert_eq!(new_stops.load(Ordering::SeqCst), 1);
}

#[test]
fn unknown_visitor_never_stages_audio() {
    let manager = VisitorSessionManager::new(Arc::new(Rt1WorkspaceService::new()));
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    assert!(
        manager
            .enqueue_media_fragment("missing", &sender, vec![1_u8, 2])
            .is_err()
    );
    assert!(receiver.try_recv().is_err());
}

#[test]
fn revoked_visitor_never_stages_audio() {
    let manager = VisitorSessionManager::new(Arc::new(Rt1WorkspaceService::new()));
    let session = Arc::new(ManagedVisitor {
        egress: Arc::new(VisitorEgressState::default()),
        authority: Rt1VisitorSession::inert_for_revocation_test(),
        revoked: AtomicBool::new(true),
        stopped: AtomicBool::new(false),
        fast_revoke: Arc::new(|| {}),
        stop: Arc::new(|| {}),
    });
    manager.sessions.lock().insert("revoked".into(), session);
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    assert!(
        manager
            .enqueue_media_fragment("revoked", &sender, vec![1_u8, 2])
            .is_err()
    );
    assert!(receiver.try_recv().is_err());
}

#[test]
fn no_visitor_session_without_canonical_publication() {
    let manager = VisitorSessionManager::new(Arc::new(Rt1WorkspaceService::new()));
    assert!(manager.open("0".repeat(64), || {}).is_err());
    assert_eq!(manager.active_count(), 0);
    assert!(manager.authorize("not-a-session").is_err());
    assert!(manager.begin_provider_egress("not-a-session").is_err());
}

#[test]
fn failed_buffered_computation_still_checks_revocation() {
    let rechecked = AtomicBool::new(false);
    let result: Result<(), String> =
        finish_buffered(Err("buffered computation failed".into()), || {
            rechecked.store(true, Ordering::SeqCst);
            Err("visitor session revoked".into())
        });
    assert!(rechecked.load(Ordering::SeqCst));
    assert_eq!(result.unwrap_err(), "visitor session revoked");
}

#[test]
fn failed_computation_preserves_error_when_authority_is_valid() {
    let result: Result<(), String> = finish_buffered(Err("computation failed".into()), || Ok(()));
    assert_eq!(result.unwrap_err(), "computation failed");
}

#[test]
fn successful_computation_cannot_bypass_final_revocation() {
    let result = finish_buffered(Ok("unreleased"), || Err("visitor session revoked".into()));
    assert_eq!(result.unwrap_err(), "visitor session revoked");
}

#[test]
fn owner_revocation_dispatches_actual_remote_operation_cancel_http() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;
    use vpr_provider_local_open_source::operation_client::RemoteOperationClient;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let request_count = Arc::new(AtomicUsize::new(0));
    let server_count = Arc::clone(&request_count);
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut buf = [0_u8; 4096];
        loop {
            let n = socket.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..n]);
            if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let request = String::from_utf8_lossy(&bytes);
        assert!(request.starts_with("DELETE /v1/avatar/operations/owner-revoked-1 "));
        server_count.fetch_add(1, Ordering::SeqCst);
        let reply = r#"{"operation_id":"owner-revoked-1","generation":1,"session_id":"remote-1","audio_stopped":true,"video_stopped":true}"#;
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            reply.len(),
            reply
        )
        .unwrap();
    });

    let client =
        RemoteOperationClient::new(&endpoint, "worker-secret", Duration::from_secs(2)).unwrap();
    let manager = VisitorSessionManager::new(Arc::new(Rt1WorkspaceService::new()));
    let remote = Arc::new(
        crate::rt1_remote_media_revocation::Rt1RemoteMediaRevocation::new(
            client,
            "owner-revoked-1".into(),
            1,
        )
        .unwrap(),
    );
    let callback_remote = Arc::clone(&remote);
    manager.sessions.lock().insert(
        "owner-session".into(),
        Arc::new(ManagedVisitor {
            egress: Arc::new(VisitorEgressState::default()),
            authority: Rt1VisitorSession::inert_for_revocation_test(),
            revoked: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            fast_revoke: Arc::new(|| {}),
            stop: Arc::new(move || {
                let _ = callback_remote.revoke_confirmed();
            }),
        }),
    );
    manager.revoke_all();
    server.join().unwrap();
    assert_eq!(request_count.load(Ordering::SeqCst), 1);
    assert!(remote.is_revoked());
    assert!(remote.is_stop_confirmed());
    assert_eq!(manager.active_count(), 0);
    manager.revoke_all();
    assert_eq!(request_count.load(Ordering::SeqCst), 1);
}
