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
fn pending_start_reserves_identity_before_remote_create() {
    let pending = Mutex::new(std::collections::HashSet::new());
    let sessions = Mutex::new(HashMap::<String, Arc<u8>>::new());
    let first =
        super::reserve_start(&pending, &sessions, "same".into()).expect("first START must reserve");
    assert!(super::reserve_start(&pending, &sessions, "same".into()).is_none());
    drop(first);
    assert!(super::reserve_start(&pending, &sessions, "same".into()).is_some());
}

#[test]
fn failed_start_releases_pending_identity_without_erasing_registered_session() {
    let pending = Mutex::new(std::collections::HashSet::new());
    let sessions = Mutex::new(HashMap::<String, Arc<u8>>::new());
    {
        let _reservation = super::reserve_start(&pending, &sessions, "reserved".into()).unwrap();
        sessions.lock().insert("reserved".into(), Arc::new(9));
    }
    assert!(pending.lock().is_empty());
    assert!(super::reserve_start(&pending, &sessions, "reserved".into()).is_none());
    assert_eq!(**sessions.lock().get("reserved").unwrap(), 9);
}

#[test]
fn concurrent_starts_cannot_reserve_same_identity() {
    let pending = Arc::new(Mutex::new(std::collections::HashSet::new()));
    let sessions = Arc::new(Mutex::new(HashMap::<String, Arc<u8>>::new()));
    let first = super::reserve_start(&pending, &sessions, "same".into()).unwrap();
    let p = Arc::clone(&pending);
    let s = Arc::clone(&sessions);
    let worker = std::thread::spawn(move || {
        super::reserve_start(&p, &s, "same".into()).is_none()
    });
    assert!(worker.join().unwrap());
    drop(first);
}

#[test]
fn denied_browser_delivery_removes_only_confirmed_remote_stop() {
    let sessions = Mutex::new(HashMap::new());
    let conversation = Arc::new(21_u8);
    sessions.lock().insert("hidden-start".into(), Arc::clone(&conversation));

    assert_eq!(
        super::finalize_denied_delivery(
            &sessions,
            &conversation,
            Err("remote STOP timed out".into()),
            true,
        ),
        Err("remote STOP timed out".into())
    );
    assert!(sessions.lock().contains_key("hidden-start"));

    assert_eq!(
        super::finalize_denied_delivery(&sessions, &conversation, Ok(()), false),
        Err("RT1_REMOTE_VIDEO_STOP_UNCONFIRMED".into())
    );
    assert!(sessions.lock().contains_key("hidden-start"));

    super::finalize_denied_delivery(&sessions, &conversation, Ok(()), true).unwrap();
    assert!(sessions.lock().is_empty());
}

#[test]
fn denied_delivery_late_stop_cannot_remove_replacement() {
    let sessions = Mutex::new(HashMap::new());
    let old = Arc::new(11_u8);
    let replacement = Arc::new(12_u8);
    sessions.lock().insert("same".into(), Arc::clone(&replacement));
    super::finalize_denied_delivery(&sessions, &old, Ok(()), true).unwrap();
    assert!(Arc::ptr_eq(sessions.lock().get("same").unwrap(), &replacement));
}

#[test]
fn late_revoked_turn_stop_keeps_retry_handle_until_both_senders_confirmed() {
    let sessions = Arc::new(Mutex::new(HashMap::new()));
    let conversation = Arc::new(31_u8);
    sessions.lock().insert("late-turn".into(), Arc::clone(&conversation));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker_sessions = Arc::clone(&sessions);
    let worker_conversation = Arc::clone(&conversation);
    let worker = std::thread::spawn(move || {
        // Model the slow STOP invoked after the final turn authority check.
        started_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        super::finalize_denied_delivery(
            &worker_sessions,
            &worker_conversation,
            Err("remote video sender still active".into()),
            true,
        )
    });
    started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(sessions.try_lock().is_some(), "remote STOP blocked the HTTP registry");
    release_tx.send(()).unwrap();
    assert_eq!(
        worker.join().unwrap(),
        Err("remote video sender still active".into())
    );
    assert!(Arc::ptr_eq(
        sessions.lock().get("late-turn").unwrap(),
        &conversation
    ));
    assert_eq!(
        super::finalize_denied_delivery(&sessions, &conversation, Ok(()), false),
        Err("RT1_REMOTE_VIDEO_STOP_UNCONFIRMED".into())
    );
    assert!(sessions.lock().contains_key("late-turn"));
    super::finalize_denied_delivery(&sessions, &conversation, Ok(()), true).unwrap();
    assert!(sessions.lock().is_empty());
}
