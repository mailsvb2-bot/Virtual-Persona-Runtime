//! Canonical PCM16 speech payload for the private Echo worker.
//! Never accepts text, tokens, provider selection or browser-originated audio.
use std::fmt::Write as _;
use serde_json::{Value, json};
use vpr_integration::{ProviderError, ProviderErrorKind};

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
