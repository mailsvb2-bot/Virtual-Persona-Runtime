//! Canonical PCM16 speech payload for the private Echo worker.
//! Never accepts text, tokens, provider selection or browser-originated audio.
use std::fmt::Write as _;
use std::sync::Arc;

use serde_json::{Value, json};
use vpr_integration::{
    CancellationProbe, ProviderError, ProviderErrorKind, RealtimeAvatarPcmStream,
    RealtimeAvatarSession, RealtimeAvatarTransport,
};

use super::{
    DidAgentStreamsAvatar,
    echo::DidEchoRegistry,
    provider_error::{cancelled, policy_denied},
};

/// Encodes a bounded, already-authorized PCM16 buffer for the private worker.
///
/// # Errors
/// Rejects malformed frames and oversized, unsupported audio.
pub(crate) fn pcm_payload(
    pcm: &[u8],
    sample_rate_hz: u32,
    channels: u16,
) -> Result<Value, ProviderError> {
    if pcm.is_empty()
        || pcm.len() > 8 * 1024 * 1024 - 44
        || !matches!(channels, 1 | 2)
        || !(8_000..=96_000).contains(&sample_rate_hz)
        || pcm.len() % (usize::from(channels) * 2) != 0
    {
        return Err(invalid_response());
    }
    let mut encoded = String::with_capacity(pcm.len() * 2);
    for byte in pcm {
        write!(&mut encoded, "{byte:02x}").map_err(|_| invalid_response())?;
    }
    Ok(json!({"command": "pcm", "pcm_hex": encoded,
        "sample_rate_hz": sample_rate_hz, "channels": channels}))
}

fn invalid_response() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::InvalidResponse,
        retryable: false,
    }
}

pub(crate) fn send_private_pcm(
    adapter: &DidAgentStreamsAvatar,
    session: &RealtimeAvatarSession,
    pcm: &[u8],
    sample_rate_hz: u32,
    channels: u16,
    cancellation: &dyn CancellationProbe,
) -> Result<(), ProviderError> {
    DidAgentStreamsAvatar::ensure_active(cancellation)?;
    DidAgentStreamsAvatar::validate_session(session)?;
    if !matches!(session.transport, RealtimeAvatarTransport::LiveKit { .. })
        || !adapter
            .echo_sessions
            .contains(&session.provider_session_id)?
    {
        return Err(policy_denied());
    }
    let result = adapter
        .echo_backend
        .as_ref()
        .ok_or_else(policy_denied)?
        .speak_pcm(
            &session.provider_session_id,
            pcm,
            sample_rate_hz,
            channels,
            cancellation,
        );
    // A backend may return after STOP or cancellation. Never acknowledge
    // that stale publication as successful, even if the backend did.
    DidAgentStreamsAvatar::ensure_active(cancellation)?;
    if !adapter
        .echo_sessions
        .contains(&session.provider_session_id)?
    {
        return Err(policy_denied());
    }
    result
}

/// A private PCM stream retains the same Echo registration as the parent adapter.
/// A successful backend call is never acknowledged after STOP or cancellation.
/// The backend remains responsible for interrupting in-flight network I/O.
pub(super) struct RevocableEchoPcmStream {
    pub(super) inner: Box<dyn RealtimeAvatarPcmStream>,
    pub(super) sessions: Arc<DidEchoRegistry>,
    pub(super) session_id: String,
    pub(super) terminated: bool,
}

impl RevocableEchoPcmStream {
    fn check(&mut self, cancellation: &dyn CancellationProbe) -> Result<(), ProviderError> {
        if self.terminated {
            return Err(policy_denied());
        }
        if cancellation.is_cancelled() {
            let _ = self.abort();
            return Err(cancelled());
        }
        match self.sessions.contains(&self.session_id) {
            Ok(true) => Ok(()),
            result => {
                // Loss of registry authority is itself a reason to fence the
                // private utterance, including a poisoned registry.
                let _ = self.abort();
                Err(result.err().unwrap_or_else(policy_denied))
            }
        }
    }
}

impl RealtimeAvatarPcmStream for RevocableEchoPcmStream {
    fn push_pcm(
        &mut self,
        pcm: &[u8],
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        self.check(cancellation)?;
        let result = self.inner.push_pcm(pcm, cancellation);
        self.check(cancellation)?;
        if result.is_err() {
            // A partial write can already have queued speech. Do not allow
            // a retry on the same utterance after ambiguous delivery.
            let _ = self.abort();
        }
        result
    }

    fn finish(&mut self, cancellation: &dyn CancellationProbe) -> Result<(), ProviderError> {
        self.check(cancellation)?;
        let result = self.inner.finish(cancellation);
        self.check(cancellation)?;
        if result.is_err() {
            let _ = self.abort();
        } else {
            self.terminated = true;
        }
        result
    }

    fn abort(&mut self) -> Result<(), ProviderError> {
        self.terminated = true;
        self.inner.abort()
    }
}
