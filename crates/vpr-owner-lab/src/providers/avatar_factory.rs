use std::sync::Arc;

use vpr_provider_did_agent_streams::{
    DidAgentStreamsAvatar, DidAgentStreamsConfig, EchoPythonBackend, EchoPythonConfig,
};
use vpr_provider_local_open_source::{LocalOpenSourceAvatar, LocalOpenSourceAvatarConfig};
use vpr_provider_openai_speech::{OpenAiSpeechConfig, OpenAiSpeechTts};

use super::{
    ProviderCredentialProfile, ProviderDescriptor, RealtimeAvatarPort, descriptor, optional_env,
    required_value, resolved_bool, resolved_value, select_environment_avatar_provider_with,
};

pub(super) fn build_avatar(
    profile: Option<&ProviderCredentialProfile>,
) -> Result<(Box<dyn RealtimeAvatarPort>, ProviderDescriptor), String> {
    let mut get = optional_env;
    let environment_selection = select_environment_avatar_provider_with(&mut get).map_err(|_| {
        "avatar provider environment is incomplete or ambiguous; set VPR_OWNER_LAB_AVATAR_PROVIDER explicitly"
            .to_owned()
    })?;
    let name = environment_selection
        .or_else(|| profile.map(|profile| profile.avatar_provider.to_ascii_lowercase()))
        .ok_or_else(|| {
            "avatar provider is not configured; set VPR_OWNER_LAB_AVATAR_PROVIDER or configure one provider completely"
                .to_owned()
        })?;
    match name.as_str() {
        "did" | "d-id" | "did-agent-streams" => build_did_avatar(profile),
        "local" | "local-open-source" => {
            let endpoint = required_value(
                "VPR_LOCAL_AVATAR_ENDPOINT",
                profile.and_then(|profile| profile.local_avatar_endpoint.as_deref()),
            )?;
            let api_token = required_value(
                "VPR_LOCAL_AVATAR_API_TOKEN",
                profile.and_then(|profile| profile.local_avatar_api_token.as_deref()),
            )?;
            let provider = LocalOpenSourceAvatar::new(LocalOpenSourceAvatarConfig::new(
                endpoint.clone(),
                api_token,
            ))
            .map_err(|_| "local open-source avatar provider configuration rejected".to_string())?;
            Ok((
                Box::new(provider),
                descriptor(
                    "avatar",
                    "local-open-source",
                    "realtime-worker-v1",
                    &[&endpoint, "musetalk-liveportrait-compatible"],
                ),
            ))
        }
        _ => Err(format!("unsupported avatar provider: {name}")),
    }
}

fn build_did_avatar(
    profile: Option<&ProviderCredentialProfile>,
) -> Result<(Box<dyn RealtimeAvatarPort>, ProviderDescriptor), String> {
    let endpoint = resolved_value(
        "VPR_DID_ENDPOINT",
        profile.map(|profile| profile.did_endpoint.as_str()),
    )
    .unwrap_or_else(|| "https://api.d-id.com".into());
    let api_key = required_value(
        "VPR_DID_API_KEY",
        profile.map(|profile| profile.did_api_key.as_str()),
    )?;
    let agent_id = required_value(
        "VPR_DID_AGENT_ID",
        profile.map(|profile| profile.did_agent_id.as_str()),
    )?;
    let fluent = resolved_bool(
        "VPR_DID_FLUENT",
        profile.map(|profile| profile.did_fluent),
        false,
    )?;
    let provider = DidAgentStreamsAvatar::new(
        DidAgentStreamsConfig::new(endpoint.clone(), api_key, agent_id.clone()).with_fluent(fluent),
    )
    .map_err(|_| "D-ID provider configuration rejected".to_string())?;
    let echo_enabled = resolved_bool("VPR_DID_ECHO_ENABLED", None, false)?;
    let mut backend_fingerprint = vec![endpoint.as_str(), agent_id.as_str()];
    let echo_settings = if echo_enabled {
        Some((
            required_value("VPR_DID_ECHO_PYTHON", None)?,
            shared_voice_value("VPR_VOICE_ENGINE_ENDPOINT", "VPR_DID_ECHO_TTS_ENDPOINT")?,
            shared_voice_value("VPR_VOICE_ENGINE_API_KEY", "VPR_DID_ECHO_TTS_API_KEY")?,
            shared_voice_value("VPR_VOICE_ENGINE_MODEL", "VPR_DID_ECHO_TTS_MODEL")?,
            shared_voice_value("VPR_VOICE_ENGINE_VOICE", "VPR_DID_ECHO_TTS_VOICE")?,
        ))
    } else {
        None
    };
    let provider = if let Some((python, tts_endpoint, tts_key, model, voice)) = &echo_settings {
        backend_fingerprint.extend([tts_endpoint.as_str(), model.as_str(), voice.as_str()]);
        let config = EchoPythonConfig::new(python, tts_endpoint, tts_key, model, voice)
            .map_err(|_| "D-ID Echo TTS configuration rejected".to_string())?;
        // One provider-neutral TTS authority synthesizes audio server-side.
        // Echo transports only the resulting WAV; it does not invoke its own TTS.
        let tts = OpenAiSpeechTts::new(OpenAiSpeechConfig::new(
            tts_endpoint.clone(),
            tts_key.clone(),
            model.clone(),
            voice.clone(),
        ))
        .map_err(|_| "shared voice engine configuration rejected".to_string())?;
        provider.with_echo_backend(Arc::new(
            EchoPythonBackend::new(config).with_tts_port(Arc::new(tts)),
        ))
    } else {
        provider
    };
    backend_fingerprint.push(if echo_enabled {
        "echo-private-audio-v1"
    } else if fluent {
        "fluent"
    } else {
        "legacy"
    });
    Ok((
        Box::new(provider),
        descriptor(
            "avatar",
            "did-agent-streams",
            "configured-agent",
            &backend_fingerprint,
        ),
    ))
}


/// The voice provider is selected once for the runtime rather than independently
/// per avatar. Legacy Echo configuration is accepted during migration, but a
/// conflict must fail closed instead of unpredictably choosing a voice.
fn shared_voice_value(canonical: &'static str, legacy: &'static str) -> Result<String, String> {
    resolve_shared_voice_value(canonical, optional_env(canonical), optional_env(legacy))
}

fn resolve_shared_voice_value(
    canonical: &str,
    preferred: Option<String>,
    legacy: Option<String>,
) -> Result<String, String> {
    match (preferred, legacy) {
        (Some(value), Some(old_value)) if value != old_value => Err(format!(
            "conflicting shared voice engine and legacy Echo setting for {canonical}"
        )),
        (Some(value), _) | (None, Some(value)) => Ok(value),
        (None, None) => Err(format!("shared voice engine setting {canonical} is required")),
    }
}

#[cfg(test)]
mod shared_voice_tests {
    use super::resolve_shared_voice_value;

    #[test]
    fn canonical_voice_engine_is_usable_without_legacy_echo_settings() {
        assert_eq!(
            resolve_shared_voice_value("VPR_VOICE_ENGINE_VOICE", Some("voice-a".into()), None),
            Ok("voice-a".into())
        );
    }

    #[test]
    fn legacy_voice_settings_remain_usable_during_migration() {
        assert_eq!(
            resolve_shared_voice_value("VPR_VOICE_ENGINE_MODEL", None, Some("model-a".into())),
            Ok("model-a".into())
        );
    }

    #[test]
    fn mismatched_voice_engine_config_fails_closed() {
        assert!(resolve_shared_voice_value(
            "VPR_VOICE_ENGINE_ENDPOINT",
            Some("https://a.example".into()),
            Some("https://b.example".into()),
        ).is_err());
        assert!(resolve_shared_voice_value("VPR_VOICE_ENGINE_VOICE", None, None).is_err());
    }
}
