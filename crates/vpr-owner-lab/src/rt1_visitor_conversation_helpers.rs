//! RT1 visitor admission, operation identity, failure prioritization and PCM validation.
use crate::rt1_remote_media_revocation::Rt1RemoteMediaRevocation;
use crate::rt1_visitor_session_manager::VisitorSessionManager;
use super::{Arc, LabSignalBundle, LiveVisitorInterrupt, OwnerLabEngine};
use crate::state::OwnerLabStartRequest;

/// Admit visitor CREATE/offer through the same publication-scoped provider
/// egress lease used by text and streaming voice. The lease remains live
/// until the canonical interruptible runtime startup returns.
pub(super) fn start_engine_with_provider_egress(
    manager: &VisitorSessionManager,
    session_id: &str,
    engine: &mut OwnerLabEngine,
    consent: bool,
    interrupt: Arc<LiveVisitorInterrupt>,
) -> Result<LabSignalBundle, String> {
    let _egress = manager.begin_provider_egress(session_id)?;
    let startup_interrupt = Arc::clone(&interrupt);
    let started = engine.start_visitor_with_interrupt(
        OwnerLabStartRequest { consent },
        move |handle| {
            *startup_interrupt.turn.lock() = Some(handle.clone());
            if startup_interrupt.cancelled.load(std::sync::atomic::Ordering::Acquire) {
                let _ = handle.interrupt();
            }
        },
    );
    *interrupt.turn.lock() = None;
    started.map_err(|error| error.code().to_owned())
}

/// Compensate a failed CREATE without claiming a local close is a remote receipt.
pub(super) fn cleanup_failed_start(
    manager: &VisitorSessionManager,
    session_id: &str,
    engine: &mut OwnerLabEngine,
    remote: Option<&Rt1RemoteMediaRevocation>,
    original_error: String,
) -> String {
    let cleanup = engine
        .visitor_avatar_stop_handle()
        .map(|_| engine.stop_visitor_provider_confirmed());
    let _ = manager.close(session_id);
    if remote.is_some_and(|worker| worker.revoke_confirmed().is_err()) {
        return "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED".into();
    }
    if matches!(cleanup, Some(Err(_))) {
        return "RT1_PROVIDER_START_FAILED_STOP_UNCONFIRMED".into();
    }
    original_error
}

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
/// Recheck the authenticated worker after visitor authorization and close the
/// just-opened local session if the renderer or worker admission is unavailable.
pub(super) fn require_worker_ready(
    manager: &VisitorSessionManager,
    id: &str,
    remote: Option<&Rt1RemoteMediaRevocation>,
) -> Result<(), String> {
    if let Some(reason) = remote.and_then(|worker| worker.publication_ready().err()) {
        return manager.close(id).and(Err(reason.to_owned()));
    }
    Ok(())
}

pub(super) fn validate_microphone_audio(pcm: &[u8]) -> Result<(), String> {
    // Match the existing RT0 voice input duration and PCM framing limits.
    const MAX_VOICE_BYTES: usize = 16_000 * 2 * 30;
    if pcm.is_empty() || pcm.len() > MAX_VOICE_BYTES || pcm.len() % 2 != 0 {
        return Err("RT1_INVALID_MICROPHONE_AUDIO".into());
    }
    Ok(())
}

/** Preserve the exact worker OFFER signaling and ICE credentials without
 * introducing a second signaling path or advertising voice capabilities. */
pub(super) fn diagnostic_signal_bundle(
    offer: vpr_provider_local_open_source::operation_client::RemoteOffer,
) -> crate::state::LabSignalBundle {
    crate::state::LabSignalBundle {
        evidence_session_sequence: 1,
        transport: crate::state::LabRealtimeTransport::WebRtc {
            offer: crate::state::LabSessionDescription {
                kind: offer.kind,
                sdp: offer.sdp,
            },
            ice_servers: offer
                .ice_servers
                .into_iter()
                .map(|server| crate::state::LabIceServer {
                    urls: server.urls,
                    username: server.username,
                    credential: server.credential,
                })
                .collect(),
        },
        capabilities: Vec::new(),
        client_control: None,
    }
}
