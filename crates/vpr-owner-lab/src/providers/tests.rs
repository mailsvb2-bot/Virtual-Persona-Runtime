use super::{
    credential_source_allows_secure_store_with, matching_llm_profile, matching_stt_profile,
    openai_compatible_provider_name, provider_config_complete_with,
};
use crate::ProviderCredentialProfile;

fn profile() -> ProviderCredentialProfile {
    ProviderCredentialProfile::canonical_rt0(
        "did-secret",
        "did-agent",
        "deepgram-secret".into(),
        "deepseek-secret".into(),
    )
}

#[test]
fn credential_source_policy_is_explicit_and_fail_closed() {
    assert_eq!(credential_source_allows_secure_store_with(None), Ok(true));
    assert_eq!(
        credential_source_allows_secure_store_with(Some("auto")),
        Ok(true)
    );
    assert_eq!(
        credential_source_allows_secure_store_with(Some("environment")),
        Ok(false)
    );
    assert!(credential_source_allows_secure_store_with(Some("mixed")).is_err());
}

#[test]
fn complete_explicit_environment_does_not_need_secure_store() {
    let values = [
        "VPR_DID_API_KEY",
        "VPR_DID_AGENT_ID",
        "VPR_OWNER_LAB_STT_PROVIDER",
        "VPR_OWNER_LAB_STT_ENDPOINT",
        "VPR_OWNER_LAB_STT_API_KEY",
        "VPR_OWNER_LAB_STT_MODEL",
        "VPR_OWNER_LAB_LLM_PROVIDER",
        "VPR_OWNER_LAB_LLM_ENDPOINT",
        "VPR_OWNER_LAB_LLM_API_KEY",
        "VPR_OWNER_LAB_LLM_MODEL",
    ];
    assert!(provider_config_complete_with(true, |name| {
        values.contains(&name).then(|| "configured".into())
    }));
}

#[test]
fn stored_avatar_selection_can_supply_local_worker_without_did_env() {
    let mut profile = profile();
    profile
        .select_local_avatar("https://avatar.example.test", "worker-secret")
        .unwrap();
    let (avatar, descriptor) = super::build_avatar(Some(&profile)).unwrap();
    assert_eq!(descriptor.provider, "local-open-source");
    assert_eq!(avatar.descriptor().provider, "local-open-source");
}

#[test]
fn local_avatar_environment_is_complete_without_any_did_configuration() {
    let values = [
        ("VPR_LOCAL_AVATAR_ENDPOINT", "https://avatar.example.test"),
        ("VPR_LOCAL_AVATAR_API_TOKEN", "worker-token"),
        ("VPR_OWNER_LAB_STT_PROVIDER", "deepgram"),
        ("VPR_OWNER_LAB_STT_ENDPOINT", "https://stt.example.test"),
        ("VPR_OWNER_LAB_STT_API_KEY", "stt-secret"),
        ("VPR_OWNER_LAB_STT_MODEL", "nova-3"),
        ("VPR_OWNER_LAB_LLM_PROVIDER", "deepseek"),
        ("VPR_OWNER_LAB_LLM_ENDPOINT", "https://llm.example.test"),
        ("VPR_OWNER_LAB_LLM_API_KEY", "llm-secret"),
        ("VPR_OWNER_LAB_LLM_MODEL", "deepseek-flash"),
    ];
    assert!(provider_config_complete_with(true, |name| {
        values
            .iter()
            .find_map(|(key, value)| (*key == name).then(|| (*value).to_owned()))
    }));
}

#[test]
fn ambiguous_avatar_environment_requires_explicit_selection() {
    let values = [
        "VPR_DID_API_KEY",
        "VPR_DID_AGENT_ID",
        "VPR_LOCAL_AVATAR_ENDPOINT",
        "VPR_LOCAL_AVATAR_API_TOKEN",
    ];
    assert!(!provider_config_complete_with(false, |name| {
        values.contains(&name).then(|| "configured".into())
    }));
}

#[test]
fn local_avatar_environment_can_replace_did_without_changing_voice_stack() {
    let values = [
        ("VPR_OWNER_LAB_AVATAR_PROVIDER", "local-open-source"),
        ("VPR_LOCAL_AVATAR_ENDPOINT", "https://avatar.example.test"),
        ("VPR_LOCAL_AVATAR_API_TOKEN", "worker-token"),
        ("VPR_OWNER_LAB_STT_PROVIDER", "deepgram"),
        ("VPR_OWNER_LAB_STT_ENDPOINT", "https://stt.example.test"),
        ("VPR_OWNER_LAB_STT_API_KEY", "stt-secret"),
        ("VPR_OWNER_LAB_STT_MODEL", "nova-3"),
        ("VPR_OWNER_LAB_LLM_PROVIDER", "deepseek"),
        ("VPR_OWNER_LAB_LLM_ENDPOINT", "https://llm.example.test"),
        ("VPR_OWNER_LAB_LLM_API_KEY", "llm-secret"),
        ("VPR_OWNER_LAB_LLM_MODEL", "deepseek-flash"),
    ];
    assert!(provider_config_complete_with(true, |name| {
        values
            .iter()
            .find_map(|(key, value)| (*key == name).then(|| (*value).to_owned()))
    }));
}

#[test]
fn partial_voice_environment_requires_secure_store_fallback() {
    let values = [
        "VPR_DID_API_KEY",
        "VPR_DID_AGENT_ID",
        "VPR_OWNER_LAB_STT_PROVIDER",
        "VPR_OWNER_LAB_LLM_PROVIDER",
    ];
    assert!(!provider_config_complete_with(true, |name| {
        values.contains(&name).then(|| "configured".into())
    }));
}

#[test]
fn secure_profile_is_used_only_for_the_matching_provider_identity() {
    let profile = profile();
    assert!(matching_stt_profile("deepgram", Some(&profile)).is_some());
    assert!(matching_stt_profile("openai", Some(&profile)).is_none());
    assert!(matching_llm_profile("deepseek", Some(&profile)).is_some());
    assert!(matching_llm_profile("anthropic", Some(&profile)).is_none());
}

#[test]
fn every_voice_profile_change_invalidates_provider_state_fingerprint() {
    use crate::{VoiceProfile, VoiceProfileStatus};
    let baseline = VoiceProfile {
        schema_version: "vpr-voice-profile-0.1".into(),
        profile_id: "interim-qwen3-serena".into(),
        version: 1,
        status: VoiceProfileStatus::Interim,
        provider: "metrotrance-qwen-preset".into(),
        model: "metrotrance-qwen-preset".into(),
        voice: "Serena".into(),
        rights_ref_sha256: None,
    };
    let descriptor = super::descriptor(
        "tts",
        "metrotrance-qwen-preset",
        "metrotrance-qwen-preset",
        &[
            "http://127.0.0.1:8897/v1/audio/speech",
            "metrotrance-qwen-preset",
            "Serena",
        ],
    );
    let digest = |voice: &VoiceProfile| {
        super::bind_tts_descriptor(descriptor.clone(), voice)
            .unwrap()
            .configuration_fingerprint_sha256
    };
    let original = digest(&baseline);
    let mut changed = baseline.clone();
    changed.version += 1;
    assert_ne!(original, digest(&changed));
    changed = baseline.clone();
    changed.status = VoiceProfileStatus::Candidate;
    assert_ne!(original, digest(&changed));
    changed = baseline.clone();
    changed.voice = "Ryan".into();
    assert_ne!(original, digest(&changed));
    changed = baseline.clone();
    changed.provider = "openai-speech".into();
    assert_ne!(original, digest(&changed));
    changed = baseline.clone();
    changed.model = "another-model".into();
    assert_ne!(original, digest(&changed));
    changed = baseline.clone();
    changed.status = VoiceProfileStatus::Accepted;
    changed.rights_ref_sha256 = Some("b".repeat(64));
    assert_ne!(original, digest(&changed));
}

#[test]
fn strict_release_voice_configuration_is_fail_closed_and_integrated_speech_remains_valid() {
    use crate::{VoiceProfile, VoiceProfileStatus};
    let mut profile = VoiceProfile {
        schema_version: "vpr-voice-profile-0.1".into(),
        profile_id: "interim-qwen3-serena".into(),
        version: 1,
        status: VoiceProfileStatus::Interim,
        provider: "metrotrance-qwen-preset".into(),
        model: "metrotrance-qwen-preset".into(),
        voice: "Serena".into(),
        rights_ref_sha256: None,
    };
    let strict = super::validate_strict_voice_selection;
    assert!(strict(None, None, false).is_ok());
    assert!(strict(Some("metrotrance-qwen-preset"), Some(&profile), false).is_ok());
    assert!(strict(Some("metrotrance-qwen-preset"), None, false).is_err());
    assert!(strict(Some("metrotrance-qwen-preset"), Some(&profile), true).is_err());
    profile.status = VoiceProfileStatus::Candidate;
    assert!(strict(Some("metrotrance-qwen-preset"), Some(&profile), false).is_err());
    profile.status = VoiceProfileStatus::Accepted;
    profile.rights_ref_sha256 = Some("a".repeat(64));
    assert!(strict(Some("metrotrance-qwen-preset"), Some(&profile), false).is_ok());
}

#[test]
fn deepseek_keeps_its_provider_identity_over_openai_compatible_transport() {
    assert_eq!(
        openai_compatible_provider_name("deepseek"),
        Some("deepseek")
    );
    assert_eq!(
        openai_compatible_provider_name("openai"),
        Some("openai-compatible")
    );
    assert_eq!(openai_compatible_provider_name("anthropic"), None);
}

#[test]
fn yandex_and_proxyapi_use_one_canonical_openai_compatible_llm_port() {
    assert_eq!(openai_compatible_provider_name("yandex"), Some("yandex"));
    assert_eq!(
        openai_compatible_provider_name("proxyapi-deepseek"),
        Some("proxyapi-deepseek")
    );
    assert_eq!(openai_compatible_provider_name("unknown"), None);
}
