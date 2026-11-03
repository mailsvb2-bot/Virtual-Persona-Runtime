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
