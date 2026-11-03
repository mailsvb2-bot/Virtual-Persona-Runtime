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
