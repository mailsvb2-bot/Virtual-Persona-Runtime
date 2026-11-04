//! Separate credential verification from launch readiness for Expressive D-ID.
//! Saving valid D-ID keys must work before an Echo/TTS route is configured.
//! The actual launch/probe remains fail-closed without private Echo.
use std::env;
use vpr_owner_lab::{ProviderCredentialProfile, StoredTtsCredential};
use vpr_provider_did_agent_streams::EchoPythonConfig;

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

fn legacy_private_echo_route_ready() -> bool {
    // Kept for the existing server-owned EchoPythonConfig::new path. Do not
    // conflate this legacy backend with canonical PCM-only TTS synthesis.
    let values = [
        "VPR_DID_ECHO_PYTHON",
        "VPR_DID_ECHO_TTS_ENDPOINT",
        "VPR_DID_ECHO_TTS_API_KEY",
        "VPR_DID_ECHO_TTS_MODEL",
        "VPR_DID_ECHO_TTS_VOICE",
    ];
    let [Some(python), Some(endpoint), Some(api_key), Some(model), Some(voice)] =
        values.map(|name| env::var(name).ok())
    else {
        return false;
    };
    EchoPythonConfig::new(python, endpoint, api_key, model, voice).is_ok()
}

fn launch_echo_denied(
    presenter: &str,
    purpose: DidProbePurpose,
    canonical_ready: bool,
    legacy_ready: bool,
    explicit_enabled: Option<&str>,
) -> bool {
    if purpose == DidProbePurpose::CredentialSetup || !presenter.eq_ignore_ascii_case("expressive") {
        return false;
    }
    let echo_enabled = explicit_enabled
        .or(if canonical_ready { Some("true") } else { None });
    !(canonical_ready || legacy_ready) || expressive_echo_preflight_denied(presenter, echo_enabled)
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
    let canonical_ready = private_voice_route_ready(profile);
    let legacy_ready = legacy_private_echo_route_ready();
    let override_value = env::var("VPR_DID_ECHO_ENABLED").ok();
    launch_echo_denied(
        presenter,
        purpose,
        canonical_ready,
        legacy_ready,
        override_value.as_deref(),
    )
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
    fn launch_authorizes_both_canonical_and_existing_legacy_private_echo_routes() {
        assert!(!launch_echo_denied("expressive", DidProbePurpose::LaunchReadiness, true, false, None));
        assert!(!launch_echo_denied("expressive", DidProbePurpose::LaunchReadiness, false, true, Some("true")));
        assert!(launch_echo_denied("expressive", DidProbePurpose::LaunchReadiness, false, true, None));
        assert!(launch_echo_denied("expressive", DidProbePurpose::LaunchReadiness, true, false, Some("false")));
        assert!(launch_echo_denied("expressive", DidProbePurpose::LaunchReadiness, false, false, Some("true")));
        assert!(!launch_echo_denied("expressive", DidProbePurpose::CredentialSetup, false, false, Some("false")));
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
