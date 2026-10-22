use parking_lot::Mutex as ParkingMutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tiny_http::Request;

use crate::{HttpResponse, error_response, header_value, http_json::read_body, json_response};

const MAX_REFERENCE_BODY_BYTES: u64 = 12 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(crate) struct ReferenceMetadata {
    kind: String,
    media_type: String,
    bytes: u64,
    sha256: String,
    raw_retained: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct ReferenceSnapshot {
    voice: Option<ReferenceMetadata>,
    appearance: Option<ReferenceMetadata>,
}

#[derive(Default)]
pub(crate) struct ReferenceIntakeState {
    snapshot: ParkingMutex<ReferenceSnapshot>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClearReferenceBody {
    kind: String,
}

impl ReferenceIntakeState {
    pub(crate) fn snapshot(&self) -> ReferenceSnapshot {
        self.snapshot.lock().clone()
    }

    fn replace(&self, metadata: ReferenceMetadata) {
        let mut snapshot = self.snapshot.lock();
        match metadata.kind.as_str() {
            "voice" => snapshot.voice = Some(metadata),
            "appearance" => snapshot.appearance = Some(metadata),
            _ => unreachable!("validated reference kind"),
        }
    }

    fn clear(&self, kind: &str) -> bool {
        let mut snapshot = self.snapshot.lock();
        match kind {
            "voice" => snapshot.voice.take().is_some(),
            "appearance" => snapshot.appearance.take().is_some(),
            _ => false,
        }
    }
}

pub(crate) fn snapshot_response(state: &ReferenceIntakeState) -> HttpResponse {
    json_response(200, &state.snapshot())
}

pub(crate) fn intake_response(
    request: &mut Request,
    state: &ReferenceIntakeState,
) -> Result<HttpResponse, HttpResponse> {
    let kind = required_header(request, "X-VPR-Reference-Kind")?.to_ascii_lowercase();
    let media_type = required_header(request, "X-VPR-Reference-Media-Type")?.to_ascii_lowercase();
    let consent = required_header(request, "X-VPR-Reference-Consent")?;

    if consent != "confirmed" {
        return Err(error_response(403, "CONSENT_REQUIRED"));
    }
    validate_kind_and_media_type(&kind, &media_type)?;

    let body = read_body(request, MAX_REFERENCE_BODY_BYTES)?;
    if body.is_empty() {
        return Err(error_response(400, "INVALID_INPUT"));
    }

    let metadata = ReferenceMetadata {
        kind,
        media_type,
        bytes: body.len() as u64,
        sha256: sha256_hex(&body),
        raw_retained: false,
    };
    // The raw biometric/reference bytes intentionally die here. RT0 only keeps
    // sanitized metadata proving local record/upload intake; durable biometric
    // retention belongs to a later consent/rights/Vault-backed production path.
    state.replace(metadata.clone());
    Ok(json_response(201, &metadata))
}

pub(crate) fn clear_response(
    request: &mut Request,
    state: &ReferenceIntakeState,
) -> Result<HttpResponse, HttpResponse> {
    let body = crate::http_json::parse_json::<ClearReferenceBody>(request)?;
    let kind = body.kind.trim().to_ascii_lowercase();
    if !matches!(kind.as_str(), "voice" | "appearance") {
        return Err(error_response(400, "INVALID_INPUT"));
    }
    let cleared = state.clear(&kind);
    Ok(json_response(
        200,
        &serde_json::json!({"ok": true, "kind": kind, "cleared": cleared}),
    ))
}

fn required_header<'a>(request: &'a Request, name: &'static str) -> Result<&'a str, HttpResponse> {
    header_value(request, name)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| error_response(400, "INVALID_INPUT"))
}

fn validate_kind_and_media_type(kind: &str, media_type: &str) -> Result<(), HttpResponse> {
    let allowed = match kind {
        "voice" => media_type.starts_with("audio/"),
        "appearance" => media_type.starts_with("image/") || media_type.starts_with("video/"),
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(error_response(415, "REFERENCE_MEDIA_TYPE_DENIED"))
    }
}

fn sha256_hex(body: &[u8]) -> String {
    let digest = Sha256::digest(body);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_and_appearance_media_types_are_fail_closed() {
        assert!(validate_kind_and_media_type("voice", "audio/webm").is_ok());
        assert!(validate_kind_and_media_type("voice", "image/png").is_err());
        assert!(validate_kind_and_media_type("appearance", "image/jpeg").is_ok());
        assert!(validate_kind_and_media_type("appearance", "video/webm").is_ok());
        assert!(validate_kind_and_media_type("appearance", "audio/wav").is_err());
        assert!(validate_kind_and_media_type("unknown", "audio/wav").is_err());
    }

    #[test]
    fn snapshot_replaces_by_kind_without_raw_payload() {
        let state = ReferenceIntakeState::default();
        state.replace(ReferenceMetadata {
            kind: "voice".into(),
            media_type: "audio/webm".into(),
            bytes: 3,
            sha256: sha256_hex(b"abc"),
            raw_retained: false,
        });
        let snapshot = state.snapshot();
        let voice = snapshot.voice.expect("voice metadata");
        assert_eq!(voice.bytes, 3);
        assert_eq!(
            voice.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(!voice.raw_retained);
        assert!(snapshot.appearance.is_none());
    }

    #[test]
    fn clear_is_idempotent() {
        let state = ReferenceIntakeState::default();
        assert!(!state.clear("voice"));
        state.replace(ReferenceMetadata {
            kind: "voice".into(),
            media_type: "audio/webm".into(),
            bytes: 1,
            sha256: sha256_hex(b"x"),
            raw_retained: false,
        });
        assert!(state.clear("voice"));
        assert!(!state.clear("voice"));
    }
}
