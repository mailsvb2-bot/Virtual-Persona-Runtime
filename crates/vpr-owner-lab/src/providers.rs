use std::env;

use sha2::{Digest, Sha256};
use vpr_integration::{LlmPort, RealtimeAvatarPort, SttPort, TtsPort};
use vpr_provider_anthropic::{AnthropicConfig, AnthropicLlm};
use vpr_provider_deepgram_stt::{DeepgramStt, DeepgramSttConfig};
use vpr_provider_did_agent_streams::{DidAgentStreamsAvatar, DidAgentStreamsConfig};
use vpr_provider_elevenlabs_tts::{ElevenLabsTts, ElevenLabsTtsConfig};
use vpr_provider_gemini::{GeminiConfig, GeminiLlm};
use vpr_provider_openai_compatible::{OpenAiCompatibleConfig, OpenAiCompatibleLlm};
use vpr_provider_openai_speech::{OpenAiSpeechConfig, OpenAiSpeechTts};
use vpr_provider_openai_transcription::{OpenAiTranscriptionConfig, OpenAiTranscriptionStt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDescriptor {
    pub provider: String,
    pub model_or_representation: String,
    pub configuration_fingerprint_sha256: String,
}

pub struct ProviderBundle {
    pub avatar: Box<dyn RealtimeAvatarPort>,
    pub stt: Option<Box<dyn SttPort>>,
    pub llm: Option<Box<dyn LlmPort>>,
    pub tts: Option<Box<dyn TtsPort>>,
    pub avatar_descriptor: ProviderDescriptor,
    pub stt_descriptor: Option<ProviderDescriptor>,
    pub llm_descriptor: Option<ProviderDescriptor>,
    pub tts_descriptor: Option<ProviderDescriptor>,
}

impl ProviderBundle {
    /// Builds the exact provider composition used by Owner Lab from environment variables.
    ///
    /// API keys are consumed by concrete adapters but are never retained in descriptors.
    ///
    /// # Errors
    /// Returns a redacted configuration error when required settings are missing or rejected.
    pub fn from_env(require_voice: bool) -> Result<Self, String> {
        let did_endpoint =
            env::var("VPR_DID_ENDPOINT").unwrap_or_else(|_| "https://api.d-id.com".into());
        let did_api_key = required_env("VPR_DID_API_KEY")?;
        let did_agent_id = required_env("VPR_DID_AGENT_ID")?;
        let avatar = DidAgentStreamsAvatar::new(DidAgentStreamsConfig::new(
            did_endpoint.clone(),
            did_api_key,
            did_agent_id.clone(),
        ))
        .map_err(|_| "D-ID provider configuration rejected".to_string())?;
        let avatar_descriptor = descriptor(
            "avatar",
            "did-agent-streams",
            "configured-agent",
            &[&did_endpoint, &did_agent_id],
        );

        let stt_name = optional_env("VPR_OWNER_LAB_STT_PROVIDER");
        let llm_name = optional_env("VPR_OWNER_LAB_LLM_PROVIDER");
        let tts_name = optional_env("VPR_OWNER_LAB_TTS_PROVIDER");
        let (stt, llm, stt_descriptor, llm_descriptor) = match (stt_name, llm_name) {
            (None, None) if !require_voice => (None, None, None, None),
            (Some(stt_name), Some(llm_name)) => {
                let (stt, stt_descriptor) = build_stt(&stt_name)?;
                let (llm, llm_descriptor) = build_llm(&llm_name)?;
                (
                    Some(stt),
                    Some(llm),
                    Some(stt_descriptor),
                    Some(llm_descriptor),
                )
            }
            _ => {
                return Err(
                    "voice mode requires both VPR_OWNER_LAB_STT_PROVIDER and VPR_OWNER_LAB_LLM_PROVIDER"
                        .into(),
                );
            }
        };
        let (tts, tts_descriptor) = match tts_name {
            Some(name) => {
                let (tts, descriptor) = build_tts(&name)?;
                (Some(tts), Some(descriptor))
            }
            None if require_voice => {
                return Err("live proof requires VPR_OWNER_LAB_TTS_PROVIDER".into());
            }
            None => (None, None),
        };
        Ok(Self {
            avatar: Box::new(avatar),
            stt,
            llm,
            tts,
            avatar_descriptor,
            stt_descriptor,
            llm_descriptor,
            tts_descriptor,
        })
    }
}

fn build_stt(name: &str) -> Result<(Box<dyn SttPort>, ProviderDescriptor), String> {
    let endpoint = required_env("VPR_OWNER_LAB_STT_ENDPOINT")?;
    let api_key = required_env("VPR_OWNER_LAB_STT_API_KEY")?;
    let model = required_env("VPR_OWNER_LAB_STT_MODEL")?;
    let (canonical, provider): (&str, Box<dyn SttPort>) = match name {
        "openai" | "openai-transcription" => (
            "openai-transcription",
            Box::new(
                OpenAiTranscriptionStt::new(OpenAiTranscriptionConfig::new(
                    endpoint.clone(),
                    api_key,
                    model.clone(),
                ))
                .map_err(|_| "STT provider configuration rejected")?,
            ),
        ),
        "deepgram" => (
            "deepgram",
            Box::new(
                DeepgramStt::new(DeepgramSttConfig::new(
                    endpoint.clone(),
                    api_key,
                    model.clone(),
                ))
                .map_err(|_| "STT provider configuration rejected")?,
            ),
        ),
        _ => return Err(format!("unsupported STT provider: {name}")),
    };
    Ok((
        provider,
        descriptor("stt", canonical, &model, &[&endpoint, &model]),
    ))
}

fn build_llm(name: &str) -> Result<(Box<dyn LlmPort>, ProviderDescriptor), String> {
    let endpoint = required_env("VPR_OWNER_LAB_LLM_ENDPOINT")?;
    let api_key = required_env("VPR_OWNER_LAB_LLM_API_KEY")?;
    let model = required_env("VPR_OWNER_LAB_LLM_MODEL")?;
    let (canonical, provider): (&str, Box<dyn LlmPort>) = match name {
        "openai" | "openai-compatible" => (
            "openai-compatible",
            Box::new(
                OpenAiCompatibleLlm::new(OpenAiCompatibleConfig::new(
                    endpoint.clone(),
                    api_key,
                    model.clone(),
                ))
                .map_err(|_| "LLM provider configuration rejected")?,
            ),
        ),
        "anthropic" => (
            "anthropic",
            Box::new(
                AnthropicLlm::new(AnthropicConfig::new(
                    endpoint.clone(),
                    api_key,
                    model.clone(),
                ))
                .map_err(|_| "LLM provider configuration rejected")?,
            ),
        ),
        "gemini" => (
            "gemini",
            Box::new(
                GeminiLlm::new(GeminiConfig::new(endpoint.clone(), api_key, model.clone()))
                    .map_err(|_| "LLM provider configuration rejected")?,
            ),
        ),
        _ => return Err(format!("unsupported LLM provider: {name}")),
    };
    Ok((
        provider,
        descriptor("llm", canonical, &model, &[&endpoint, &model]),
    ))
}

fn build_tts(name: &str) -> Result<(Box<dyn TtsPort>, ProviderDescriptor), String> {
    let endpoint = required_env("VPR_OWNER_LAB_TTS_ENDPOINT")?;
    let api_key = required_env("VPR_OWNER_LAB_TTS_API_KEY")?;
    let model = required_env("VPR_OWNER_LAB_TTS_MODEL")?;
    let voice = required_env("VPR_OWNER_LAB_TTS_VOICE")?;
    let (canonical, provider): (&str, Box<dyn TtsPort>) = match name {
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
    Ok((
        provider,
        descriptor(
            "tts",
            canonical,
            &format!("{model}/{voice}"),
            &[&endpoint, &model, &voice],
        ),
    ))
}

fn descriptor(
    role: &str,
    provider: &str,
    model_or_representation: &str,
    configuration_parts: &[&str],
) -> ProviderDescriptor {
    let mut hasher = Sha256::new();
    hasher.update(b"vpr-provider-state-v1\nrole=");
    hasher.update(role.as_bytes());
    hasher.update(b"\nprovider=");
    hasher.update(provider.as_bytes());
    for part in configuration_parts {
        hasher.update(b"\npart=");
        hasher.update(part.as_bytes());
    }
    ProviderDescriptor {
        provider: provider.into(),
        model_or_representation: model_or_representation.into(),
        configuration_fingerprint_sha256: format!("{:x}", hasher.finalize()),
    }
}

fn optional_env(name: &'static str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
}

fn required_env(name: &'static str) -> Result<String, String> {
    env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("required environment variable {name} is not set"))
}
