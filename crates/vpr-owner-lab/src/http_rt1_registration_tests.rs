use super::{RegistrationFailure, finalize_registered_stop, register_visitor};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn revoked_registration_retains_retry_handle_until_remote_receipt() {
    let sessions = Mutex::new(HashMap::new());
    let conversation = Arc::new(7_u8);
    assert_eq!(
        register_visitor(&sessions, "revoked", &conversation, |_| false),
        Err(RegistrationFailure::Revoked)
    );
    assert!(Arc::ptr_eq(
        sessions.lock().get("revoked").unwrap(),
        &conversation
    ));
    assert!(
        finalize_registered_stop(
            &sessions,
            "revoked",
            &conversation,
            Err("RT1_REMOTE_MEDIA_STOP_UNCONFIRMED".into()),
            true,
        )
        .is_err()
    );
    assert!(sessions.lock().contains_key("revoked"));
    assert_eq!(
        finalize_registered_stop(&sessions, "revoked", &conversation, Ok(()), false),
        Err("RT1_REMOTE_VIDEO_STOP_UNCONFIRMED".into())
    );
    assert!(sessions.lock().contains_key("revoked"));
    finalize_registered_stop(&sessions, "revoked", &conversation, Ok(()), true).unwrap();
    assert!(sessions.lock().is_empty());
}

#[test]
fn revoke_after_successful_registration_preserves_retryable_stop() {
    let sessions = Mutex::new(HashMap::new());
    let conversation = Arc::new(17_u8);
    assert_eq!(
        register_visitor(&sessions, "late-revoke", &conversation, |_| true),
        Ok(())
    );
    // A pause can win after the registration check and before the HTTP
    // response. Unconfirmed remote STOP must retain the only retry handle.
    assert_eq!(
        finalize_registered_stop(
            &sessions,
            "late-revoke",
            &conversation,
            Err("remote sender still active".into()),
            true,
        ),
        Err("remote sender still active".into())
    );
    assert!(Arc::ptr_eq(
        sessions.lock().get("late-revoke").unwrap(),
        &conversation
    ));
    finalize_registered_stop(&sessions, "late-revoke", &conversation, Ok(()), true).unwrap();
    assert!(sessions.lock().is_empty());
}

#[test]
fn late_stop_cannot_remove_replacement_registration() {
    let sessions = Mutex::new(HashMap::new());
    let old = Arc::new(1_u8);
    let replacement = Arc::new(2_u8);
    sessions
        .lock()
        .insert("reused".into(), Arc::clone(&replacement));
    finalize_registered_stop(&sessions, "reused", &old, Ok(()), true).unwrap();
    assert!(Arc::ptr_eq(
        sessions.lock().get("reused").unwrap(),
        &replacement
    ));
}

#[test]
fn stalled_post_registration_authorization_never_holds_http_registry() {
    let sessions = Arc::new(Mutex::new(HashMap::new()));
    let worker_sessions = Arc::clone(&sessions);
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        register_visitor(&worker_sessions, "slow", &Arc::new(1_u8), |_| {
            started_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            false
        })
    });
    started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    let lock_available = sessions.try_lock().is_some();
    release_tx.send(()).unwrap();
    assert!(lock_available, "slow provider STOP held all HTTP sessions");
    assert_eq!(worker.join().unwrap(), Err(RegistrationFailure::Revoked));
    // The HTTP registration survives until STOP is independently proven.
    assert!(sessions.lock().contains_key("slow"));
    let conversation = sessions.lock().get("slow").unwrap().clone();
    assert!(finalize_registered_stop(&sessions, "slow", &conversation, Ok(()), true).is_ok());
    assert!(sessions.lock().is_empty());
}

#[test]
fn revoked_candidate_cannot_erase_concurrent_replacement() {
    let sessions = Mutex::new(HashMap::new());
    let candidate = Arc::new(1_u8);
    let replacement = Arc::new(2_u8);
    let result = register_visitor(&sessions, "same", &candidate, |_| {
        sessions
            .lock()
            .insert("same".into(), Arc::clone(&replacement));
        false
    });
    assert_eq!(result, Err(RegistrationFailure::Revoked));
    assert!(Arc::ptr_eq(
        sessions.lock().get("same").unwrap(),
        &replacement
    ));
}

#[test]
fn collision_never_invokes_provider_authorization() {
    let sessions = Mutex::new(HashMap::new());
    let existing = Arc::new(1_u8);
    sessions.lock().insert("same".into(), Arc::clone(&existing));
    let candidate = Arc::new(2_u8);
    let result = register_visitor(&sessions, "same", &candidate, |_| {
        panic!("colliding registration must not invoke provider")
    });
    assert_eq!(result, Err(RegistrationFailure::Collision));
    assert!(Arc::ptr_eq(sessions.lock().get("same").unwrap(), &existing));
}

#[test]
fn rejected_provider_result_cannot_be_tagged_as_authorized_http_delivery() {
    let routed = super::guarded_reply(Err(super::error_response(409, "RT1_VISITOR_TEXT_DENIED")));
    assert!(routed.delivery.is_none());
    assert_eq!(routed.response.status_code().0, 409);
}

#[test]
fn failed_startup_cleanup_survives_lost_stop_and_reconciles_actual_receipt() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use vpr_provider_local_open_source::operation_client::RemoteOperationClient;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        for attempt in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut buffer = [0_u8; 2048];
            let mut request = Vec::new();
            loop {
                let count = socket.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                if request.windows(4).any(|chunk| chunk == b"\r\n\r\n") {
                    break;
                }
            }
            assert!(request.starts_with(b"DELETE /v1/avatar/operations/lost-offer "));
            let (status, body) = if attempt == 0 {
                (
                    "202 Accepted",
                    r#"{"operation_id":"lost-offer","stop_confirmed":false}"#,
                )
            } else {
                (
                    "200 OK",
                    r#"{"operation_id":"lost-offer","generation":1,"session_id":"real-worker-sender","audio_stopped":true,"video_stopped":true}"#,
                )
            };
            write!(
                socket,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    let worker_client =
        RemoteOperationClient::new(&endpoint, "secret", Duration::from_secs(3)).unwrap();
    let remote = Arc::new(
        super::Rt1RemoteMediaRevocation::new(worker_client, "lost-offer".into(), 1).unwrap(),
    );
    let weak = Arc::downgrade(&remote);
    let state = super::Rt1HttpState::default();
    state
        .register_remote("never-delivered-to-browser".into(), Arc::clone(&remote))
        .unwrap();
    drop(remote);
    // Only the HTTP-owned cleanup registry retains this operation after the
    // failed startup discards the visitor conversation and its session ID.
    let retained = weak.upgrade().expect("lost cleanup handle");
    assert!(retained.revoke_confirmed().is_err(), "HTTP 202 is not STOP");
    drop(retained);
    assert!(weak.upgrade().is_some(), "uncertain STOP lost retry handle");
    state.reconcile_remote_stops();
    assert!(
        weak.upgrade().is_none(),
        "confirmed STOP retained stale handle"
    );
    worker.join().unwrap();
}

#[test]
fn registered_remote_revoke_wakes_one_existing_cleanup_reconciler() {
    use std::net::TcpListener;
    use vpr_provider_local_open_source::operation_client::RemoteOperationClient;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let client = RemoteOperationClient::new(&endpoint, "secret", Duration::from_secs(3)).unwrap();
    let remote = Arc::new(
        super::Rt1RemoteMediaRevocation::new(client, "wake-owner-stop".into(), 1).unwrap(),
    );
    let state = super::Rt1HttpState::default();
    state
        .register_remote("retry".into(), Arc::clone(&remote))
        .unwrap();
    let (sequence, _) = &*state.cleanup_notify;
    let before = *sequence.lock();
    remote.request_revoke();
    assert_ne!(
        *sequence.lock(),
        before,
        "revocation did not wake STOP reconciler"
    );
    assert!(remote.is_revoked());
    assert!(!remote.is_stop_confirmed());
    // Network STOP must run only in the already-registered reconciler.
    assert!(
        listener.accept().is_err(),
        "owner revoke performed blocking HTTP"
    );
}

#[test]
fn duplicate_start_id_is_denied_before_provider_create() {
    let state = Arc::new(super::Rt1HttpState::default());
    let first = super::reserve_start(&state, "same-id".into())
        .expect("first START must reserve a retry identity");
    let other = Arc::clone(&state);
    let attempted = std::thread::spawn(move || {
        super::reserve_start(&other, "same-id".into()).is_none()
    });
    assert!(
        attempted.join().unwrap(),
        "second START could reuse a pending remote CREATE identity"
    );
    drop(first);
    assert!(
        super::reserve_start(&state, "same-id".into()).is_some(),
        "failure must release the local reservation"
    );
}

#[test]
fn unresolved_remote_stop_target_cannot_be_reused_by_start() {
    use vpr_provider_local_open_source::operation_client::RemoteOperationClient;
    let state = super::Rt1HttpState::default();
    let client = RemoteOperationClient::new(
        "http://127.0.0.1:1", "test-secret", Duration::from_secs(1),
    )
    .unwrap();
    let remote = Arc::new(
        super::Rt1RemoteMediaRevocation::new(client, "pending-remote".into(), 1).unwrap(),
    );
    state.remote_cleanup.lock().insert("same-id".into(), remote);
    assert!(
        super::reserve_start(&state, "same-id".into()).is_none(),
        "unconfirmed STOP identity must not be overwritten by CREATE"
    );
}
