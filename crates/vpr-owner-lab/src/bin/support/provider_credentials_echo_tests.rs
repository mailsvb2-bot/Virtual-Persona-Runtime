// Exercises the existing fail-closed Expressive Echo preflight.
use super::expressive_echo_preflight_denied;

#[test]
fn expressive_without_private_echo_is_rejected_before_ui_launch() {
    assert!(expressive_echo_preflight_denied("expressive", None));
    assert!(expressive_echo_preflight_denied(
        "Expressive",
        Some("false")
    ));
    assert!(!expressive_echo_preflight_denied(
        "expressive",
        Some("true")
    ));
    assert!(!expressive_echo_preflight_denied(
        "Expressive",
        Some(" True ")
    ));
    assert!(expressive_echo_preflight_denied(
        "Expressive",
        Some(" false ")
    ));
}

#[test]
fn securely_saved_tts_profile_counts_as_configured_voice_route() {
    use super::{StoredTtsCredential, stored_voice_route_ready};
    assert!(!stored_voice_route_ready(None));
    let good = StoredTtsCredential {
        provider: "piper".into(),
        endpoint: "http://127.0.0.1:8765".into(),
        api_key: "local-only".into(),
        model: "ru_RU".into(),
        voice: "voice".into(),
    };
    assert!(stored_voice_route_ready(Some(&good)));
    let mut incomplete = good;
    incomplete.endpoint.clear();
    assert!(!stored_voice_route_ready(Some(&incomplete)));
}

#[test]
fn legacy_presenters_remain_compatible_without_echo() {
    assert!(!expressive_echo_preflight_denied("legacy", None));
    assert!(!expressive_echo_preflight_denied("talk", None));
}
