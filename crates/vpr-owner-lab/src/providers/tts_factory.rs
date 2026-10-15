//! Provider-neutral compatibility gate and configured speech adapter construction.
//! A valid TTS provider is not sufficient: the selected avatar must accept private PCM.
use super::{ProviderDescriptor, descriptor, required_value};
use crate::StoredTtsCredential;
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

pub(super) fn build_tts(
    name: &str,
    stored: Option<&StoredTtsCredential>,
) -> Result<(Box<dyn TtsPort>, ProviderDescriptor), String> {
    let stored = stored.filter(|credential| credential.provider.eq_ignore_ascii_case(name));
    build_tts_configured(
        name,
        &required_value(
            "VPR_OWNER_LAB_TTS_ENDPOINT",
            stored.map(|item| item.endpoint.as_str()),
        )?,
        required_value(
            "VPR_OWNER_LAB_TTS_API_KEY",
            stored.map(|item| item.api_key.as_str()),
        )?,
        &required_value(
            "VPR_OWNER_LAB_TTS_MODEL",
            stored.map(|item| item.model.as_str()),
        )?,
        &required_value(
            "VPR_OWNER_LAB_TTS_VOICE",
            stored.map(|item| item.voice.as_str()),
        )?,
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
    if matches!(
        name,
        "piper"
            | "windows-sapi"
            | "metrotrance-qwen"
            | "metrotrance-qwen-preset"
            | "metrotrance-chatterbox"
    ) && !is_loopback_speech_endpoint(endpoint)
    {
        return Err("local TTS must use a loopback HTTP speech endpoint".into());
    }
    if name == "windows-sapi" && (model != "windows-sapi" || voice != "local") {
        return Err("Windows SAPI requires model=windows-sapi and voice=local".into());
    }
    if name == "piper" && (model != "piper" || voice != "local") {
        return Err("Piper requires model=piper and voice=local".into());
    }
    let provider: Box<dyn TtsPort> = match name {
        "piper"
        | "windows-sapi"
        | "metrotrance-qwen"
        | "metrotrance-qwen-preset"
        | "metrotrance-chatterbox"
        | "openai-speech" => Box::new(
            OpenAiSpeechTts::new(
                OpenAiSpeechConfig::new(
                    endpoint.to_owned(),
                    api_key,
                    model.to_owned(),
                    voice.to_owned(),
                )
                .with_provider_name(name)
                .with_stream_pcm(name == "piper"),
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
    // A fresh private loopback port is transport detail, not provider identity.
    let fingerprint_endpoint = if is_loopback_speech_endpoint(endpoint) {
        "http://127.0.0.1:<private-loopback>/v1/audio/speech"
    } else {
        endpoint
    };
    Ok((
        provider,
        descriptor("tts", name, model, &[fingerprint_endpoint, model, voice]),
    ))
}

/// Fail closed: an opaque URL containing credentials, DNS aliases, fragments or
/// redirectable paths must not make a local speech engine an external egress.
fn is_loopback_speech_endpoint(endpoint: &str) -> bool {
    let host_port_path = endpoint
        .strip_prefix("http://127.0.0.1:")
        .or_else(|| endpoint.strip_prefix("http://localhost:"));
    let Some(host_port_path) = host_port_path else {
        return false;
    };
    let Some((port, path)) = host_port_path.split_once('/') else {
        return false;
    };
    port.parse::<u16>().is_ok_and(|port| port > 0) && path == "v1/audio/speech"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_loopback_port_does_not_change_tts_fingerprint() {
        let build = |endpoint: &str, voice: &str| {
            build_tts_configured("metrotrance-qwen", endpoint, "token".into(), "model", voice)
                .unwrap()
                .1
                .configuration_fingerprint_sha256
        };
        let a = build("http://127.0.0.1:8897/v1/audio/speech", "local");
        let b = build("http://127.0.0.1:49152/v1/audio/speech", "local");
        let c = build("http://127.0.0.1:8897/v1/audio/speech", "other");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    use std::sync::Arc;
    use vpr_provider_did_agent_streams::{
        DidAgentStreamsAvatar, DidAgentStreamsConfig, EchoPythonBackend, EchoPythonConfig,
    };
    use vpr_provider_local_open_source::{LocalOpenSourceAvatar, LocalOpenSourceAvatarConfig};

    #[test]
    fn piper_uses_server_owned_openai_wire_without_cloning_a_provider() {
        let (tts, descriptor) = build_tts_configured(
            "piper",
            "http://127.0.0.1:8897/v1/audio/speech",
            "test-local-bearer".into(),
            "piper",
            "local",
        )
        .expect("Piper must be a first-class local TTS provider");
        assert_eq!(descriptor.provider, "piper");
        assert_eq!(tts.descriptor().provider, "piper");
    }

    #[test]
    fn local_speech_never_allows_non_loopback_or_ambiguous_endpoints() {
        for endpoint in [
            "https://example.test/v1/audio/speech",
            "http://example.test:8897/v1/audio/speech",
            "http://127.0.0.1:8897@evil.test/v1/audio/speech",
            "http://127.0.0.1:8897/v1/audio/speech?remote=1",
            "http://127.0.0.1:8897/other",
            "http://localhost:0/v1/audio/speech",
        ] {
            for provider in [
                "piper",
                "windows-sapi",
                "metrotrance-qwen",
                "metrotrance-qwen-preset",
                "metrotrance-chatterbox",
            ] {
                let result = build_tts_configured(
                    provider,
                    endpoint,
                    "private-key".into(),
                    provider,
                    "local",
                );
                assert!(result.is_err(), "unsafe {provider} endpoint: {endpoint}");
            }
        }
        assert!(is_loopback_speech_endpoint(
            "http://localhost:8897/v1/audio/speech"
        ));
    }

    #[test]
    fn qwen_preset_uses_existing_server_owned_tts_port_only() {
        let (tts, descriptor) = build_tts_configured(
            "metrotrance-qwen-preset",
            "http://127.0.0.1:8897/v1/audio/speech",
            "private-token".into(),
            "metrotrance-qwen-preset",
            "local",
        )
        .expect("preset voice must reuse existing PCM publication path");
        assert_eq!(tts.descriptor().provider, "metrotrance-qwen-preset");
        assert_eq!(descriptor.provider, "metrotrance-qwen-preset");
        assert!(
            build_tts_configured(
                "metrotrance-qwen-preset",
                "https://external.example.test/v1/audio/speech",
                "private-token".into(),
                "metrotrance-qwen-preset",
                "local",
            )
            .is_err()
        );
    }

    #[test]
    fn native_windows_voice_uses_private_tts_and_rejects_wrong_identity() {
        let (tts, descriptor) = build_tts_configured(
            "windows-sapi",
            "http://127.0.0.1:8897/v1/audio/speech",
            "private-token".into(),
            "windows-sapi",
            "local",
        )
        .unwrap();
        assert_eq!(tts.descriptor().provider, "windows-sapi");
        assert_eq!(descriptor.provider, "windows-sapi");
        assert!(
            build_tts_configured(
                "windows-sapi",
                "http://127.0.0.1:8897/v1/audio/speech",
                "private-token".into(),
                "windows-sapi",
                "speaker-custom",
            )
            .is_err()
        );
    }

    #[test]
    fn piper_mismatched_model_or_voice_fails_before_session_start() {
        assert!(
            build_tts_configured(
                "piper",
                "http://127.0.0.1:8897/v1/audio/speech",
                "private".into(),
                "metrotrance-qwen",
                "local"
            )
            .is_err()
        );
        assert!(
            build_tts_configured(
                "piper",
                "http://127.0.0.1:8897/v1/audio/speech",
                "private".into(),
                "piper",
                "speaker-clone"
            )
            .is_err()
        );
    }

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
