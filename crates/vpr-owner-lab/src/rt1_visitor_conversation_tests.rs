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
fn staged_streaming_cannot_call_provider_or_enqueue_without_stop_capability() {
    use vpr_provider_local_open_source::{LocalOpenSourceAvatar, LocalOpenSourceAvatarConfig};

    // The configured adapter is real, but this negative test must never
    // contact it: no independent STOP handle has been established.
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
    };
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    assert_eq!(
        conversation
            .voice_turn_streaming_staged(vec![0_u8; 320], &sender)
            .unwrap_err(),
        "RT1_VOICE_PROVIDER_EGRESS_NOT_AUTHORIZED"
    );
    assert!(receiver.try_recv().is_err());
}

#[test]
fn drop_cleanup_never_waits_for_a_busy_provider_engine() {
    use std::time::{Duration, Instant};
    use vpr_provider_local_open_source::{LocalOpenSourceAvatar, LocalOpenSourceAvatarConfig};

    let provider = LocalOpenSourceAvatar::new(LocalOpenSourceAvatarConfig::new(
        "http://127.0.0.1:1",
        "test-only",
    ))
    .unwrap();
    let engine = Mutex::new(OwnerLabEngine::new(Box::new(provider), true).unwrap());
    // A provider turn may hold this mutex while external I/O is stalled.
    let _busy = engine.lock();
    let before = Instant::now();
    try_stop_uncontended(&engine);
    assert!(
        before.elapsed() < Duration::from_millis(250),
        "Drop cleanup must not wait for the provider engine mutex"
    );
    // Even after the quiescence check, a concurrent turn may acquire the
    // engine mutex. Explicit close must fail closed, not wait indefinitely.
    let before_close = Instant::now();
    assert_eq!(
        stop_if_uncontended(&engine),
        Err("RT1_PROVIDER_EGRESS_NOT_QUIESCENT".into())
    );
    assert!(
        before_close.elapsed() < Duration::from_millis(250),
        "Explicit close must not wait for a busy provider engine"
    );
}

#[test]
fn revoked_stop_without_provider_handle_remains_retryable() {
    let interrupt = LiveVisitorInterrupt::default();
    interrupt.stop();
    assert!(interrupt.cancelled.load(Ordering::Acquire));
    assert!(!interrupt.stop_in_flight.load(Ordering::Acquire));
    assert!(!interrupt.stop_confirmed.load(Ordering::Acquire));
    // A late CREATE may install the stop-only handle after initial revoke.
    // The missing handle must never permanently consume the STOP attempt.
    interrupt.stop();
    assert!(!interrupt.stop_in_flight.load(Ordering::Acquire));
    assert!(!interrupt.stop_confirmed.load(Ordering::Acquire));
}

#[test]
fn confirmed_provider_cleanup_suppresses_close_callback_stop_dispatch() {
    // The revoked-during-CREATE path performs confirmed engine cleanup
    // before manager.close(). A confirmed result must not schedule another
    // asynchronous STOP through the visitor callback.
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
    // Simulate the synchronous interruption owning the STOP fence while
    // an owner revoke concurrently invokes its asynchronous callback.
    interrupt.stop_in_flight.store(true, Ordering::Release);
    interrupt.stop();
    assert!(interrupt.cancelled.load(Ordering::Acquire));
    assert!(interrupt.stop_in_flight.load(Ordering::Acquire));
    assert!(!interrupt.stop_confirmed.load(Ordering::Acquire));
    // The synchronous owner releases the fence after the provider returns.
    // An uncertain outcome remains retryable rather than being confirmed.
    interrupt.stop_in_flight.store(false, Ordering::Release);
    interrupt.stop();
    assert!(!interrupt.stop_in_flight.load(Ordering::Acquire));
    assert!(!interrupt.stop_confirmed.load(Ordering::Acquire));
}

#[test]
fn close_does_not_confirm_or_duplicate_an_inflight_stop() {
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
    };
    assert_eq!(
        conversation.close(),
        Err("RT1_VISITOR_INTERRUPT_STOP_UNCONFIRMED".into())
    );
    assert!(interrupt.stop_in_flight.load(Ordering::Acquire));
    assert!(!interrupt.stop_confirmed.load(Ordering::Acquire));
}
