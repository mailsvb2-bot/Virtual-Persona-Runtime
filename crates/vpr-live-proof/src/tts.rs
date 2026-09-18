use std::env;

use sha2::{Digest, Sha256};
use vpr_integration::TtsPort;
use vpr_provider_elevenlabs_tts::{ElevenLabsTts, ElevenLabsTtsConfig};
use vpr_provider_openai_speech::{OpenAiSpeechConfig, OpenAiSpeechTts};

pub(crate) struct PreparedTtsProbe {
    pub provider: Box<dyn TtsPort>,
    pub provider_name: String,
    pub model_or_representation: String,
    pub configuration_fingerprint_sha256: String,
}

pub(crate) fn build_tts_probe_from_env() -> Result<PreparedTtsProbe, String> {
    let name = required_env("VPR_OWNER_LAB_TTS_PROVIDER")?
        .trim()
        .to_ascii_lowercase();
    let endpoint = required_env("VPR_OWNER_LAB_TTS_ENDPOINT")?;
    let api_key = required_env("VPR_OWNER_LAB_TTS_API_KEY")?;
    let model = required_env("VPR_OWNER_LAB_TTS_MODEL")?;
    let voice = required_env("VPR_OWNER_LAB_TTS_VOICE")?;

    let (canonical, provider): (&str, Box<dyn TtsPort>) = match name.as_str() {
        "openai" | "openai-speech" => (
            "openai-speech",
            Box::new(
                OpenAiSpeechTts::new(OpenAiSpeechConfig::new(
                    endpoint.clone(),
                    api_key,
                    model.clone(),
                    voice.clone(),
                ))
                .map_err(|_| "TTS provider configuration rejected")?,
            ),
        ),
        "elevenlabs" => (
            "elevenlabs",
            Box::new(
                ElevenLabsTts::new(ElevenLabsTtsConfig::new(
                    endpoint.clone(),
                    api_key,
                    model.clone(),
                    voice.clone(),
                ))
                .map_err(|_| "TTS provider configuration rejected")?,
            ),
        ),
        _ => return Err(format!("unsupported TTS provider: {name}")),
    };

    Ok(PreparedTtsProbe {
        provider,
        provider_name: canonical.into(),
        model_or_representation: format!("{model}/{voice}"),
        configuration_fingerprint_sha256: fingerprint(
            canonical,
            &[&endpoint, &model, &voice],
        ),
    })
}

fn fingerprint(provider: &str, configuration_parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"vpr-provider-state-v1\nrole=tts");
    hasher.update(b"\nprovider=");
    hasher.update(provider.as_bytes());
    for part in configuration_parts {
        hasher.update(b"\npart=");
        hasher.update(part.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn required_env(name: &'static str) -> Result<String, String> {
    env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("required environment variable {name} is not set"))
}
