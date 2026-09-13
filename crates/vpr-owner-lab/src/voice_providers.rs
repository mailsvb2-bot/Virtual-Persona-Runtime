use std::env;

use vpr_integration::{LlmPort, SttPort};
use vpr_provider_anthropic::{AnthropicConfig, AnthropicLlm};
use vpr_provider_deepgram_stt::{DeepgramStt, DeepgramSttConfig};
use vpr_provider_gemini::{GeminiConfig, GeminiLlm};
use vpr_provider_openai_compatible::{OpenAiCompatibleConfig, OpenAiCompatibleLlm};
use vpr_provider_openai_transcription::{OpenAiTranscriptionConfig, OpenAiTranscriptionStt};

pub(crate) struct VoiceProviders {
    pub(crate) stt: Box<dyn SttPort>,
    pub(crate) llm: Box<dyn LlmPort>,
}

pub(crate) fn from_env() -> Result<Option<VoiceProviders>, String> {
    let stt_name = optional_env("VPR_OWNER_LAB_STT_PROVIDER");
    let llm_name = optional_env("VPR_OWNER_LAB_LLM_PROVIDER");
    match (stt_name, llm_name) {
        (None, None) => Ok(None),
        (Some(stt), Some(llm)) => Ok(Some(VoiceProviders {
            stt: build_stt(&stt)?,
            llm: build_llm(&llm)?,
        })),
        _ => Err(
            "voice mode requires both VPR_OWNER_LAB_STT_PROVIDER and VPR_OWNER_LAB_LLM_PROVIDER"
                .into(),
        ),
    }
}

fn build_stt(name: &str) -> Result<Box<dyn SttPort>, String> {
    let endpoint = required_env("VPR_OWNER_LAB_STT_ENDPOINT")?;
    let api_key = required_env("VPR_OWNER_LAB_STT_API_KEY")?;
    let model = required_env("VPR_OWNER_LAB_STT_MODEL")?;
    match name {
        "openai" | "openai-transcription" => {
            OpenAiTranscriptionStt::new(OpenAiTranscriptionConfig::new(endpoint, api_key, model))
                .map(|provider| Box::new(provider) as Box<dyn SttPort>)
                .map_err(|_| "STT provider configuration rejected".into())
        }
        "deepgram" => DeepgramStt::new(DeepgramSttConfig::new(endpoint, api_key, model))
            .map(|provider| Box::new(provider) as Box<dyn SttPort>)
            .map_err(|_| "STT provider configuration rejected".into()),
        _ => Err(format!("unsupported STT provider: {name}")),
    }
}

fn build_llm(name: &str) -> Result<Box<dyn LlmPort>, String> {
    let endpoint = required_env("VPR_OWNER_LAB_LLM_ENDPOINT")?;
    let api_key = required_env("VPR_OWNER_LAB_LLM_API_KEY")?;
    let model = required_env("VPR_OWNER_LAB_LLM_MODEL")?;
    match name {
        "openai" | "openai-compatible" => {
            OpenAiCompatibleLlm::new(OpenAiCompatibleConfig::new(endpoint, api_key, model))
                .map(|provider| Box::new(provider) as Box<dyn LlmPort>)
                .map_err(|_| "LLM provider configuration rejected".into())
        }
        "anthropic" => AnthropicLlm::new(AnthropicConfig::new(endpoint, api_key, model))
            .map(|provider| Box::new(provider) as Box<dyn LlmPort>)
            .map_err(|_| "LLM provider configuration rejected".into()),
        "gemini" => GeminiLlm::new(GeminiConfig::new(endpoint, api_key, model))
            .map(|provider| Box::new(provider) as Box<dyn LlmPort>)
            .map_err(|_| "LLM provider configuration rejected".into()),
        _ => Err(format!("unsupported LLM provider: {name}")),
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
