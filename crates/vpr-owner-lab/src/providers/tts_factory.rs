//! Provider-neutral compatibility gate and configured speech adapter construction.
//! A valid TTS provider is not sufficient: the selected avatar must accept private PCM.
use super::{ProviderDescriptor, descriptor, required_value};
use vpr_integration::{RealtimeAvatarCapability, RealtimeAvatarPort, TtsPort};
use vpr_provider_elevenlabs_tts::{ElevenLabsTts, ElevenLabsTtsConfig};
use vpr_provider_openai_speech::{OpenAiSpeechConfig, OpenAiSpeechTts};

pub(super) fn ensure_avatar_pcm_compatible(avatar: &dyn RealtimeAvatarPort) -> Result<(), String> {
    if avatar
        .capabilities()
        .supports(RealtimeAvatarCapability::AudioPcmInput)
    {
        Ok(())
    } else {
        Err("canonical TTS requires an avatar adapter with private PCM input; choose a compatible avatar provider before starting a session".into())
    }
}

pub(super) fn build_tts(name: &str) -> Result<(Box<dyn TtsPort>, ProviderDescriptor), String> {
    build_tts_configured(
        name,
        &required_value("VPR_OWNER_LAB_TTS_ENDPOINT", None)?,
        required_value("VPR_OWNER_LAB_TTS_API_KEY", None)?,
        &required_value("VPR_OWNER_LAB_TTS_MODEL", None)?,
        &required_value("VPR_OWNER_LAB_TTS_VOICE", None)?,
    )
}

pub(super) fn build_tts_fallback(
    name: &str,
) -> Result<(Box<dyn TtsPort>, ProviderDescriptor), String> {
    build_tts_configured(
        name,
        &required_value("VPR_OWNER_LAB_TTS_ALT_ENDPOINT", None)?,
        required_value("VPR_OWNER_LAB_TTS_ALT_API_KEY", None)?,
        &required_value("VPR_OWNER_LAB_TTS_ALT_MODEL", None)?,
        &required_value("VPR_OWNER_LAB_TTS_ALT_VOICE", None)?,
    )
}

fn build_tts_configured(
    name: &str,
    endpoint: &str,
    api_key: String,
    model: &str,
    voice: &str,
) -> Result<(Box<dyn TtsPort>, ProviderDescriptor), String> {
    let provider: Box<dyn TtsPort> = match name {
        "metrotrance-qwen" | "metrotrance-chatterbox" | "openai-speech" => Box::new(
            OpenAiSpeechTts::new(
                OpenAiSpeechConfig::new(
                    endpoint.to_owned(),
                    api_key,
                    model.to_owned(),
                    voice.to_owned(),
                )
                .with_provider_name(name),
            )
            .map_err(|_| "OpenAI-compatible speech provider configuration rejected")?,
        ),
        "elevenlabs" => Box::new(
            ElevenLabsTts::new(ElevenLabsTtsConfig::new(
                endpoint.to_owned(),
                api_key,
                model.to_owned(),
                voice.to_owned(),
            ))
            .map_err(|_| "ElevenLabs speech provider configuration rejected")?,
        ),
        _ => return Err(format!("unsupported TTS provider: {name}")),
    };
    if name.starts_with("metrotrance-")
        && !(endpoint.starts_with("http://127.0.0.1:") || endpoint.starts_with("http://localhost:"))
    {
        return Err("MetroTrance TTS must use a loopback endpoint".into());
    }
    Ok((
        provider,
        descriptor("tts", name, model, &[endpoint, model, voice]),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use vpr_provider_did_agent_streams::{
        DidAgentStreamsAvatar, DidAgentStreamsConfig, EchoPythonBackend, EchoPythonConfig,
    };
    use vpr_provider_local_open_source::{LocalOpenSourceAvatar, LocalOpenSourceAvatarConfig};

    #[test]
    fn rejects_unsupported_avatar_before_any_tts_or_session_side_effect() {
        let local = LocalOpenSourceAvatar::new(LocalOpenSourceAvatarConfig::new(
            "https://avatar.example.test",
            "token",
        ))
        .unwrap();
        assert!(ensure_avatar_pcm_compatible(&local).is_err());
    }

    #[test]
    fn accepts_only_explicit_echo_enabled_pcm_avatar() {
        let did = DidAgentStreamsAvatar::new(DidAgentStreamsConfig::new(
            "https://api.d-id.com",
            "test-key",
            "test-agent",
        ))
        .unwrap();
        assert!(ensure_avatar_pcm_compatible(&did).is_err());
        let pcm = did.with_echo_backend(Arc::new(EchoPythonBackend::new(
            EchoPythonConfig::pcm_only("python".to_owned()).unwrap(),
        )));
        assert!(ensure_avatar_pcm_compatible(&pcm).is_ok());
    }
}
