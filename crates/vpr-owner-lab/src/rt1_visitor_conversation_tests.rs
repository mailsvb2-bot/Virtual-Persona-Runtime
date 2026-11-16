//! Regression tests for the canonical RT1 visitor conversation.
use super::*;

mod close_outcome_tests {
    use super::finish_close;

    #[test]
    fn unconfirmed_remote_stop_overrides_successful_local_cleanup() {
        assert_eq!(
            finish_close(Err("RT1_REMOTE_MEDIA_STOP_UNCONFIRMED".into()), Ok(())),
            Err("RT1_REMOTE_MEDIA_STOP_UNCONFIRMED".into())
        );
    }

    #[test]
    fn local_failure_is_preserved_after_confirmed_remote_stop() {
        assert_eq!(
            finish_close(Ok(()), Err("LOCAL_REVOKE_FAILED".into())),
            Err("LOCAL_REVOKE_FAILED".into())
        );
    }

    #[test]
    fn close_requires_both_remote_and_local_success() {
        assert!(finish_close(Ok(()), Ok(())).is_ok());
    }
}

mod microphone_staging_tests {
    use super::validate_microphone_audio;

    #[test]
    fn canonical_microphone_frame_limits_match_rt0() {
        assert!(validate_microphone_audio(&[]).is_err());
        assert!(validate_microphone_audio(&[0_u8]).is_err());
        assert!(validate_microphone_audio(&[0_u8; 2]).is_ok());
        assert!(validate_microphone_audio(&vec![0_u8; 16_000 * 2 * 30]).is_ok());
        assert!(validate_microphone_audio(&vec![0_u8; 16_000 * 2 * 30 + 2]).is_err());
    }
}

mod rt1_voice_egress_tests {
    use super::*;

    #[test]
    fn voice_provider_egress_is_denied_without_media_fence() {
        let result: Result<LabVoiceResult, String> = unsupported_visitor_voice_egress();
        assert_eq!(
            result.unwrap_err(),
            "RT1_VOICE_PROVIDER_EGRESS_NOT_AUTHORIZED"
        );
    }

    #[test]
    fn voice_streaming_rejects_before_any_delivery() {
        let result: Result<LabVoiceResult, String> = unsupported_visitor_voice_egress();
        assert_eq!(
            result.unwrap_err(),
            "RT1_VOICE_PROVIDER_EGRESS_NOT_AUTHORIZED"
        );
    }
}

#[test]
fn text_and_streaming_voice_remain_denied_without_verified_worker_stop() {
    use vpr_provider_local_open_source::{LocalOpenSourceAvatar, LocalOpenSourceAvatarConfig};

    // This configured adapter is deliberately unreachable. Both public
    // visitor egress paths must deny before any STT/LLM/provider request.
    let provider = LocalOpenSourceAvatar::new(LocalOpenSourceAvatarConfig::new(
        "http://127.0.0.1:1",
        "test-only",
    ))
    .unwrap();
    let engine = OwnerLabEngine::new(Box::new(provider), true).unwrap();
    let manager = Arc::new(VisitorSessionManager::new(Arc::new(
        crate::rt1_workspace_service::Rt1WorkspaceService::new(),
    )));
    let conversation = Rt1VisitorConversation {
        manager,
        id: "unadmitted".into(),
        engine: Mutex::new(engine),
        interrupt: Arc::new(LiveVisitorInterrupt::default()),
        remote: None,
        verified_worker_voice: false,
    };
    assert!(!conversation.remote_media_stop_confirmed());
    assert!(!conversation.supports_private_voice());
    assert_eq!(
        conversation.text_turn("must not reach LLM").unwrap_err(),
        "RT1_VISITOR_TEXT_EGRESS_NOT_AUTHORIZED"
    );
    assert_eq!(
        conversation
            .voice_turn_streaming(vec![0_u8; 320])
            .unwrap_err(),
        "RT1_VOICE_PROVIDER_EGRESS_NOT_AUTHORIZED"
    );
}

#[test]
fn visitor_provider_cleanup_does_not_block_on_busy_engine_mutex() {
    use std::time::{Duration, Instant};
    use vpr_provider_local_open_source::{LocalOpenSourceAvatar, LocalOpenSourceAvatarConfig};

    let provider = LocalOpenSourceAvatar::new(LocalOpenSourceAvatarConfig::new(
        "http://127.0.0.1:1",
        "test-only",
    ))
    .unwrap();
    let engine = Mutex::new(OwnerLabEngine::new(Box::new(provider), true).unwrap());
    // Simulate a provider request holding the engine during revoke/close.
    let _busy = engine.lock();
    let before = Instant::now();
    assert_eq!(
        stop_if_uncontended(&engine),
        Err("RT1_PROVIDER_EGRESS_NOT_QUIESCENT".into())
    );
    assert!(
        before.elapsed() < Duration::from_millis(250),
        "close or Drop must not block on a stalled provider mutex"
    );
}

#[test]
fn revoked_stop_without_provider_handle_remains_retryable() {
    let interrupt = LiveVisitorInterrupt::default();
    interrupt.stop();
    assert!(interrupt.cancelled.load(Ordering::Acquire));
    assert!(!interrupt.stop_in_flight.load(Ordering::Acquire));
    assert!(!interrupt.stop_confirmed.load(Ordering::Acquire));
    // A late CREATE can install the handle after initial revoke.
    interrupt.stop();
    assert!(!interrupt.stop_in_flight.load(Ordering::Acquire));
    assert!(!interrupt.stop_confirmed.load(Ordering::Acquire));
}

#[test]
fn confirmed_provider_cleanup_suppresses_close_callback_stop_dispatch() {
    let interrupt = LiveVisitorInterrupt::default();
    interrupt.revoke_authority_only();
    interrupt.stop_confirmed.store(true, Ordering::Release);
    interrupt.stop();
    assert!(interrupt.cancelled.load(Ordering::Acquire));
    assert!(interrupt.stop_confirmed.load(Ordering::Acquire));
    assert!(!interrupt.stop_in_flight.load(Ordering::Acquire));
}

#[test]
fn revoke_callback_does_not_reset_an_inflight_interrupt_stop() {
    let interrupt = LiveVisitorInterrupt::default();
    // A synchronous visitor interrupt owns STOP while owner revoke arrives.
    interrupt.stop_in_flight.store(true, Ordering::Release);
    interrupt.stop();
    assert!(interrupt.cancelled.load(Ordering::Acquire));
    assert!(interrupt.stop_in_flight.load(Ordering::Acquire));
    assert!(!interrupt.stop_confirmed.load(Ordering::Acquire));
    // Once the previous attempt returns, a failed/uncertain STOP remains
    // eligible for retry and must not be fabricated as confirmed.
    interrupt.stop_in_flight.store(false, Ordering::Release);
    interrupt.stop();
    assert!(!interrupt.stop_in_flight.load(Ordering::Acquire));
    assert!(!interrupt.stop_confirmed.load(Ordering::Acquire));
}

#[test]
fn concurrent_owner_stop_prevents_duplicate_local_stop_and_false_close_receipt() {
    use vpr_provider_local_open_source::{LocalOpenSourceAvatar, LocalOpenSourceAvatarConfig};

    let provider = LocalOpenSourceAvatar::new(LocalOpenSourceAvatarConfig::new(
        "http://127.0.0.1:1",
        "test-only",
    ))
    .unwrap();
    let engine = OwnerLabEngine::new(Box::new(provider), true).unwrap();
    let manager = Arc::new(VisitorSessionManager::new(Arc::new(
        crate::rt1_workspace_service::Rt1WorkspaceService::new(),
    )));
    let interrupt = Arc::new(LiveVisitorInterrupt::default());
    interrupt.stop_in_flight.store(true, Ordering::Release);
    let conversation = Rt1VisitorConversation {
        manager,
        id: "unadmitted".into(),
        engine: Mutex::new(engine),
        interrupt: Arc::clone(&interrupt),
        remote: None,
        verified_worker_voice: false,
    };
    assert_eq!(
        conversation.close(),
        Err("RT1_VISITOR_INTERRUPT_STOP_UNCONFIRMED".into())
    );
    assert!(interrupt.stop_in_flight.load(Ordering::Acquire));
    assert!(!interrupt.stop_confirmed.load(Ordering::Acquire));
    // Drop is also forbidden from racing its engine-backed STOP against
    // the asynchronous callback that currently owns the same STOP fence.
    drop(conversation);
    assert!(interrupt.stop_in_flight.load(Ordering::Acquire));
    assert!(!interrupt.stop_confirmed.load(Ordering::Acquire));
}

#[test]
fn engine_stop_atomic_lease_rejects_callback_stop_and_preserves_unconfirmed_receipt() {
    use vpr_provider_local_open_source::{LocalOpenSourceAvatar, LocalOpenSourceAvatarConfig};
    let provider = LocalOpenSourceAvatar::new(LocalOpenSourceAvatarConfig::new(
        "http://127.0.0.1:1",
        "test-only",
    ))
    .unwrap();
    let engine = Mutex::new(OwnerLabEngine::new(Box::new(provider), true).unwrap());
    let interrupt = LiveVisitorInterrupt::default();

    // The asynchronous callback owns the same STOP lease. close/Drop must
    // reject an overlapping engine STOP, not infer success.
    interrupt.stop_in_flight.store(true, Ordering::Release);
    assert_eq!(
        interrupt_stop::stop_engine_serialized(&interrupt, &engine),
        Err("RT1_VISITOR_INTERRUPT_STOP_UNCONFIRMED".into())
    );
    assert!(interrupt.stop_in_flight.load(Ordering::Acquire));
    assert!(!interrupt.stop_confirmed.load(Ordering::Acquire));

    // Once the callback releases, engine cleanup can use the lease. A
    // successful local no-op is never an authenticated remote STOP receipt.
    interrupt.stop_in_flight.store(false, Ordering::Release);
    assert!(interrupt_stop::stop_engine_serialized(&interrupt, &engine).is_ok());
    assert!(!interrupt.stop_in_flight.load(Ordering::Acquire));
    assert!(!interrupt.stop_confirmed.load(Ordering::Acquire));
}

#[test]
fn busy_engine_stop_releases_lease_and_does_not_fake_remote_confirmation() {
    use vpr_provider_local_open_source::{LocalOpenSourceAvatar, LocalOpenSourceAvatarConfig};
    let provider = LocalOpenSourceAvatar::new(LocalOpenSourceAvatarConfig::new(
        "http://127.0.0.1:1",
        "test-only",
    ))
    .unwrap();
    let engine = Mutex::new(OwnerLabEngine::new(Box::new(provider), true).unwrap());
    let interrupt = LiveVisitorInterrupt::default();
    let _busy = engine.lock();
    assert_eq!(
        interrupt_stop::stop_engine_serialized(&interrupt, &engine),
        Err("RT1_PROVIDER_EGRESS_NOT_QUIESCENT".into())
    );
    assert!(!interrupt.stop_in_flight.load(Ordering::Acquire));
    assert!(!interrupt.stop_confirmed.load(Ordering::Acquire));
}


#[test]
fn dropping_visitor_wakes_remote_stop_reconciler_without_blocking_on_worker_http() {
    use std::net::TcpListener;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};
    use vpr_provider_local_open_source::operation_client::RemoteOperationClient;
    use vpr_provider_local_open_source::{LocalOpenSourceAvatar, LocalOpenSourceAvatarConfig};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let client =
        RemoteOperationClient::new(&endpoint, "worker-secret", Duration::from_secs(5)).unwrap();
    let remote = Arc::new(
        Rt1RemoteMediaRevocation::new(client, "drop-nonblocking".into(), 1).unwrap(),
    );
    let wakes = Arc::new(AtomicUsize::new(0));
    let observed_wakes = Arc::clone(&wakes);
    remote.install_revoke_notifier(move || {
        observed_wakes.fetch_add(1, Ordering::SeqCst);
    });
    let observed_remote = Arc::clone(&remote);

    let provider = LocalOpenSourceAvatar::new(LocalOpenSourceAvatarConfig::new(
        "http://127.0.0.1:1",
        "test-only",
    ))
    .unwrap();
    let engine = OwnerLabEngine::new(Box::new(provider), true).unwrap();
    let conversation = Rt1VisitorConversation {
        manager: Arc::new(VisitorSessionManager::new(Arc::new(
            crate::rt1_workspace_service::Rt1WorkspaceService::new(),
        ))),
        id: "unregistered-drop-test".into(),
        engine: Mutex::new(engine),
        interrupt: Arc::new(LiveVisitorInterrupt::default()),
        remote: Some(remote),
        verified_worker_voice: true,
    };

    let started = Instant::now();
    drop(conversation);
    assert!(
        started.elapsed() < Duration::from_millis(250),
        "Drop must not wait for the remote worker HTTP STOP request"
    );
    assert!(observed_remote.is_revoked());
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    assert!(!observed_remote.is_stop_confirmed());
    assert!(matches!(
        listener.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));
}
