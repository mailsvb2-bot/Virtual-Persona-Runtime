use std::sync::Arc;

use vpr_provider_did_agent_streams::{
    DidAgentStreamsAvatar, DidAgentStreamsConfig, EchoPythonBackend, EchoPythonConfig,
};
use vpr_provider_local_open_source::{LocalOpenSourceAvatar, LocalOpenSourceAvatarConfig};

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
    let pcm_tts_enabled = optional_env("VPR_OWNER_LAB_TTS_PROVIDER").is_some();
    let echo_enabled = resolved_bool("VPR_DID_ECHO_ENABLED", None, pcm_tts_enabled)?;
    if pcm_tts_enabled && !echo_enabled {
        return Err("canonical TTS requires server-owned D-ID Echo; do not disable Echo".into());
    }
    let mut backend_fingerprint = vec![endpoint.as_str(), agent_id.as_str()];
    let provider = if echo_enabled && pcm_tts_enabled {
        let python = resolved_value("VPR_DID_ECHO_PYTHON", None)
            .unwrap_or_else(|| "python".to_owned());
        let config = EchoPythonConfig::pcm_only(python)
            .map_err(|_| "PCM-only D-ID Echo configuration rejected".to_string())?;
        backend_fingerprint.push("runtime-tts-authorized-pcm");
        provider.with_echo_backend(Arc::new(EchoPythonBackend::new(config)))
    } else if echo_enabled {
        let python = required_value("VPR_DID_ECHO_PYTHON", None)?;
        let tts_endpoint = required_value("VPR_DID_ECHO_TTS_ENDPOINT", None)?;
        let tts_key = required_value("VPR_DID_ECHO_TTS_API_KEY", None)?;
        let model = required_value("VPR_DID_ECHO_TTS_MODEL", None)?;
        let voice = required_value("VPR_DID_ECHO_TTS_VOICE", None)?;
        backend_fingerprint.extend([tts_endpoint.as_str(), model.as_str(), voice.as_str()]);
        let config = EchoPythonConfig::new(python, tts_endpoint, tts_key, model, voice)
            .map_err(|_| "D-ID Echo TTS configuration rejected".to_string())?;
        provider.with_echo_backend(Arc::new(EchoPythonBackend::new(config)))
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
