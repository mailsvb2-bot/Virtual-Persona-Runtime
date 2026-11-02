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
