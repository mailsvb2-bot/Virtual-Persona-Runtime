use std::env;

use sha2::{Digest, Sha256};

use crate::provider_credentials::ProviderCredentialProfile;
#[cfg(windows)]
use crate::provider_credentials::load_provider_profile;
use vpr_integration::{LlmPort, RealtimeAvatarPort, SttPort};
use vpr_provider_anthropic::{AnthropicConfig, AnthropicLlm};
use vpr_provider_deepgram_stt::{DeepgramStt, DeepgramSttConfig};
use vpr_provider_did_agent_streams::{DidAgentStreamsAvatar, DidAgentStreamsConfig};
use vpr_provider_gemini::{GeminiConfig, GeminiLlm};
use vpr_provider_openai_compatible::{OpenAiCompatibleConfig, OpenAiCompatibleLlm};
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
    pub avatar_descriptor: ProviderDescriptor,
    pub stt_descriptor: Option<ProviderDescriptor>,
    pub llm_descriptor: Option<ProviderDescriptor>,
}

impl ProviderBundle {
    /// Builds the exact provider composition used by Owner Lab.
    ///
    /// Explicit environment variables have highest priority. On Windows, missing values fall back
    /// to the current user's VPR provider profile in Windows Credential Manager. API keys are
    /// consumed by concrete adapters but are never retained in descriptors.
    ///
    /// # Errors
    /// Returns a redacted configuration error when required settings are missing or rejected.
    pub fn from_env(require_voice: bool) -> Result<Self, String> {
        #[cfg(windows)]
        let profile = if environment_provider_config_complete(require_voice) {
            None
        } else {
            load_provider_profile()?
        };
        #[cfg(not(windows))]
        let profile: Option<ProviderCredentialProfile> = None;
        let did_endpoint = resolved_value(
            "VPR_DID_ENDPOINT",
            profile
                .as_ref()
                .map(|profile| profile.did_endpoint.as_str()),
        )
        .unwrap_or_else(|| "https://api.d-id.com".into());
        let did_api_key = required_value(
            "VPR_DID_API_KEY",
            profile.as_ref().map(|profile| profile.did_api_key.as_str()),
        )?;
        let did_agent_id = required_value(
            "VPR_DID_AGENT_ID",
            profile
                .as_ref()
                .map(|profile| profile.did_agent_id.as_str()),
        )?;
        let did_fluent = resolved_bool(
            "VPR_DID_FLUENT",
            profile.as_ref().map(|profile| profile.did_fluent),
            false,
        )?;

        let avatar = DidAgentStreamsAvatar::new(
            DidAgentStreamsConfig::new(did_endpoint.clone(), did_api_key, did_agent_id.clone())
                .with_fluent(did_fluent),
        )
        .map_err(|_| "D-ID provider configuration rejected".to_string())?;
        let avatar_descriptor = descriptor(
            "avatar",
            "did-agent-streams",
            "configured-agent",
            &[
                &did_endpoint,
                &did_agent_id,
                if did_fluent { "fluent" } else { "legacy" },
            ],
        );

        let stt_name = optional_env_lower("VPR_OWNER_LAB_STT_PROVIDER").or_else(|| {
            profile
                .as_ref()
                .map(|profile| profile.stt_provider.to_ascii_lowercase())
        });
        let llm_name = optional_env_lower("VPR_OWNER_LAB_LLM_PROVIDER").or_else(|| {
            profile
                .as_ref()
                .map(|profile| profile.llm_provider.to_ascii_lowercase())
        });
        let (stt, llm, stt_descriptor, llm_descriptor) = match (stt_name, llm_name) {
            (None, None) if !require_voice => (None, None, None, None),
            (Some(stt_name), Some(llm_name)) => {
                let (stt, stt_descriptor) =
                    build_stt(&stt_name, matching_stt_profile(&stt_name, profile.as_ref()))?;
                let (llm, llm_descriptor) =
                    build_llm(&llm_name, matching_llm_profile(&llm_name, profile.as_ref()))?;
                (
                    Some(stt),
                    Some(llm),
                    Some(stt_descriptor),
                    Some(llm_descriptor),
                )
            }
            _ => {
                return Err("voice mode requires both STT and LLM provider configuration".into());
            }
        };
        Ok(Self {
            avatar: Box::new(avatar),
            stt,
            llm,
            avatar_descriptor,
            stt_descriptor,
            llm_descriptor,
        })
    }
}

#[cfg(windows)]
fn environment_provider_config_complete(require_voice: bool) -> bool {
    provider_config_complete_with(require_voice, optional_env)
}

#[cfg(any(windows, test))]
fn provider_config_complete_with(
    require_voice: bool,
    mut get: impl FnMut(&'static str) -> Option<String>,
) -> bool {
    if get("VPR_DID_API_KEY").is_none() || get("VPR_DID_AGENT_ID").is_none() {
        return false;
    }

    let stt = get("VPR_OWNER_LAB_STT_PROVIDER");
    let llm = get("VPR_OWNER_LAB_LLM_PROVIDER");
    if stt.is_none() && llm.is_none() && !require_voice {
        return true;
    }
    if stt.is_none() || llm.is_none() {
        return false;
    }

    [
        "VPR_OWNER_LAB_STT_ENDPOINT",
        "VPR_OWNER_LAB_STT_API_KEY",
        "VPR_OWNER_LAB_STT_MODEL",
        "VPR_OWNER_LAB_LLM_ENDPOINT",
        "VPR_OWNER_LAB_LLM_API_KEY",
        "VPR_OWNER_LAB_LLM_MODEL",
    ]
    .into_iter()
    .all(|name| get(name).is_some())
}

fn matching_stt_profile<'a>(
    name: &str,
    profile: Option<&'a ProviderCredentialProfile>,
) -> Option<&'a ProviderCredentialProfile> {
    profile.filter(|profile| profile.stt_provider.eq_ignore_ascii_case(name))
}

fn matching_llm_profile<'a>(
    name: &str,
    profile: Option<&'a ProviderCredentialProfile>,
) -> Option<&'a ProviderCredentialProfile> {
    profile.filter(|profile| profile.llm_provider.eq_ignore_ascii_case(name))
}

fn build_stt(
    name: &str,
    profile: Option<&ProviderCredentialProfile>,
) -> Result<(Box<dyn SttPort>, ProviderDescriptor), String> {
    let endpoint = required_value(
        "VPR_OWNER_LAB_STT_ENDPOINT",
        profile.map(|profile| profile.stt_endpoint.as_str()),
    )?;
    let api_key = required_value(
        "VPR_OWNER_LAB_STT_API_KEY",
        profile.map(|profile| profile.stt_api_key.as_str()),
    )?;
    let model = required_value(
        "VPR_OWNER_LAB_STT_MODEL",
        profile.map(|profile| profile.stt_model.as_str()),
    )?;
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

fn build_llm(
    name: &str,
    profile: Option<&ProviderCredentialProfile>,
) -> Result<(Box<dyn LlmPort>, ProviderDescriptor), String> {
    let endpoint = required_value(
        "VPR_OWNER_LAB_LLM_ENDPOINT",
        profile.map(|profile| profile.llm_endpoint.as_str()),
    )?;
    let api_key = required_value(
        "VPR_OWNER_LAB_LLM_API_KEY",
        profile.map(|profile| profile.llm_api_key.as_str()),
    )?;
    let model = required_value(
        "VPR_OWNER_LAB_LLM_MODEL",
        profile.map(|profile| profile.llm_model.as_str()),
    )?;
    let (canonical, provider): (&str, Box<dyn LlmPort>) =
        if let Some(canonical) = openai_compatible_provider_name(name) {
            let mut config = OpenAiCompatibleConfig::new(endpoint.clone(), api_key, model.clone())
                .with_provider_name(canonical);
            if canonical == "deepseek" {
                config = config
                    .with_reasoning_effort("none")
                    .with_thinking_disabled()
                    .with_max_tokens(96);
            }
            (
                canonical,
                Box::new(
                    OpenAiCompatibleLlm::new(config)
                        .map_err(|_| "LLM provider configuration rejected")?,
                ),
            )
        } else {
            match name {
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
            }
        };
    let realtime_profile = if canonical == "deepseek" {
        "thinking=disabled;reasoning=none;max_tokens=96"
    } else {
        "provider-default"
    };
    Ok((
        provider,
        descriptor(
            "llm",
            canonical,
            &model,
            &[&endpoint, &model, realtime_profile],
        ),
    ))
}

fn openai_compatible_provider_name(name: &str) -> Option<&'static str> {
    match name {
        "openai" | "openai-compatible" => Some("openai-compatible"),
        "deepseek" => Some("deepseek"),
        _ => None,
    }
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
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn optional_env_lower(name: &'static str) -> Option<String> {
    optional_env(name).map(|value| value.to_ascii_lowercase())
}

fn resolved_value(name: &'static str, stored: Option<&str>) -> Option<String> {
    optional_env(name).or_else(|| {
        stored
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
}

fn required_value(name: &'static str, stored: Option<&str>) -> Result<String, String> {
    resolved_value(name, stored).ok_or_else(|| {
        format!(
            "required provider setting {name} is not configured; on Windows run vpr-provider-credentials set"
        )
    })
}

fn resolved_bool(name: &'static str, stored: Option<bool>, default: bool) -> Result<bool, String> {
    let Some(value) = optional_env(name) else {
        return Ok(stored.unwrap_or(default));
    };
    match value.to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("environment variable {name} must be true or false")),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        matching_llm_profile, matching_stt_profile, openai_compatible_provider_name,
        provider_config_complete_with,
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
}
