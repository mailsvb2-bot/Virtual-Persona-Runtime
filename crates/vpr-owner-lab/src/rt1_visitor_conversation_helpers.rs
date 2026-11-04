//! Pure RT1 operation identity, failure prioritization and PCM validation.
use crate::rt1_remote_media_revocation::Rt1RemoteMediaRevocation;
use crate::rt1_visitor_session_manager::VisitorSessionManager;

/// Generate the existing per-operation cancellation identity before admission.
pub(super) fn diagnostic_operation_id() -> Result<String, String> {
    let mut nonce = [0_u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| "RT1_OPERATION_ENTROPY_UNAVAILABLE".to_owned())?;
    Ok(nonce
        .iter()
        .fold(String::with_capacity(32), |mut result, octet| {
            use std::fmt::Write;
            write!(&mut result, "{octet:02x}").expect("hex formatting is infallible");
            result
        }))
}

/// Deny late-revoked diagnostic SDP unless the remote STOP is confirmed.
pub(super) fn revoked_diagnostic_offer_error(
    manager: &VisitorSessionManager,
    id: &str,
    remote: &Rt1RemoteMediaRevocation,
) -> String {
    let _ = manager.close(id);
    if remote.revoke_confirmed().is_err() {
        return "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED".into();
    }
    "RT1_VISITOR_REVOKED_DURING_OFFER".into()
}

// A remote STOP without both sender receipts must never be reported as a
// successful close, even when local engine cleanup succeeded.
pub(super) fn finish_close(
    remote_stop: Result<(), String>,
    local_stop: Result<(), String>,
) -> Result<(), String> {
    remote_stop?;
    local_stop
}

pub(super) fn validate_microphone_audio(pcm: &[u8]) -> Result<(), String> {
    // Match the existing RT0 voice input duration and PCM framing limits.
    const MAX_VOICE_BYTES: usize = 16_000 * 2 * 30;
    if pcm.is_empty() || pcm.len() > MAX_VOICE_BYTES || pcm.len() % 2 != 0 {
        return Err("RT1_INVALID_MICROPHONE_AUDIO".into());
    }
    Ok(())
}
