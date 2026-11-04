//! Separate credential verification from launch readiness for Expressive D-ID.
//! Saving valid D-ID keys must work before an Echo/TTS route is configured.
//! The actual launch/probe remains fail-closed without private Echo.
use std::env;
use vpr_owner_lab::{ProviderCredentialProfile, StoredTtsCredential};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DidProbePurpose {
    CredentialSetup,
    LaunchReadiness,
}

pub(super) fn stored_voice_route_ready(stored: Option<&StoredTtsCredential>) -> bool {
    stored.is_some_and(|tts| tts.validate().is_ok())
}

fn private_voice_route_ready(profile: &ProviderCredentialProfile) -> bool {
    let env_ready = [
        "VPR_OWNER_LAB_TTS_PROVIDER",
        "VPR_OWNER_LAB_TTS_ENDPOINT",
        "VPR_OWNER_LAB_TTS_API_KEY",
        "VPR_OWNER_LAB_TTS_MODEL",
        "VPR_OWNER_LAB_TTS_VOICE",
    ]
    .iter()
    .all(|name| env::var(name).is_ok_and(|value| !value.trim().is_empty()));
    stored_voice_route_ready(profile.tts.as_ref()) || env_ready
}

fn expressive_echo_preflight_denied(presenter: &str, echo_enabled: Option<&str>) -> bool {
    presenter.eq_ignore_ascii_case("expressive")
        && !echo_enabled.is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
}

pub(super) fn echo_preflight_denied(
    profile: &ProviderCredentialProfile,
    presenter: &str,
    purpose: DidProbePurpose,
) -> bool {
    if purpose == DidProbePurpose::CredentialSetup {
        return false;
    }
    if !presenter.eq_ignore_ascii_case("expressive") {
        return false;
    }
    let route_ready = private_voice_route_ready(profile);
    // An explicit disable always wins, and a naked enabled flag must not
    // substitute for a usable private TTS route.
    let explicit = env::var("VPR_DID_ECHO_ENABLED").ok();
    let echo = explicit
        .as_deref()
        .or(if route_ready { Some("true") } else { None });
    !route_ready || expressive_echo_preflight_denied(presenter, echo)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> ProviderCredentialProfile {
        ProviderCredentialProfile::canonical_rt0(
            "user:pass",
            "agent-id",
            "deepgram-key".into(),
            "llm-key".into(),
        )
    }

    #[test]
    fn expressive_did_keys_can_be_verified_before_tts_setup() {
        let profile = profile();
        assert!(profile.tts.is_none());
        assert!(!echo_preflight_denied(
            &profile,
            "expressive",
            DidProbePurpose::CredentialSetup
        ));
    }

    #[test]
    fn launch_requires_private_tts_and_not_only_echo_flag() {
        let profile = profile();
        // This independent route predicate is deterministic and is the
        // fail-closed foundation for launch, regardless of echo override.
        assert!(!stored_voice_route_ready(profile.tts.as_ref()));
    }

    #[test]
    fn stored_tts_must_be_complete() {
        let mut profile = profile();
        assert!(!stored_voice_route_ready(profile.tts.as_ref()));
        let good = StoredTtsCredential {
            provider: "piper".into(),
            endpoint: "http://127.0.0.1:8765".into(),
            api_key: "local-token".into(),
            model: "ru_RU".into(),
            voice: "speaker".into(),
        };
        assert!(stored_voice_route_ready(Some(&good)));
        profile.tts = Some(good);
        assert!(stored_voice_route_ready(profile.tts.as_ref()));
        profile.tts.as_mut().unwrap().endpoint.clear();
        assert!(!stored_voice_route_ready(profile.tts.as_ref()));
    }

    #[test]
    fn legacy_presenter_is_not_blocked_by_expressive_gate() {
        let profile = profile();
        assert!(!echo_preflight_denied(
            &profile,
            "legacy",
            DidProbePurpose::LaunchReadiness
        ));
    }

    #[test]
    fn bare_echo_override_is_not_a_private_voice_route() {
        // No environment mutation: this tests the conservative predicate
        // independently of process-global settings and CI concurrency.
        assert!(expressive_echo_preflight_denied("Expressive", None));
        assert!(expressive_echo_preflight_denied(
            "expressive",
            Some("false")
        ));
        assert!(!expressive_echo_preflight_denied(
            "expressive",
            Some(" True ")
        ));
    }
}
