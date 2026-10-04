use std::env;

mod avatar_selection;
mod provider_state;

#[cfg(any(windows, test))]
use avatar_selection::avatar_provider_config_complete_with;
use avatar_selection::select_environment_avatar_provider_with;
use sha2::{Digest, Sha256};

use crate::provider_credentials::ProviderCredentialProfile;
#[cfg(windows)]
use crate::provider_credentials::load_provider_profile;
use vpr_integration::{LlmPort, RealtimeAvatarPort, SttPort};
use vpr_provider_anthropic::{AnthropicConfig, AnthropicLlm};
use vpr_provider_deepgram_stt::{DeepgramStt, DeepgramSttConfig};
use vpr_provider_did_agent_streams::{DidAgentStreamsAvatar, DidAgentStreamsConfig};
use vpr_provider_gemini::{GeminiConfig, GeminiLlm};
use vpr_provider_local_open_source::{LocalOpenSourceAvatar, LocalOpenSourceAvatarConfig};
use vpr_provider_openai_compatible::{OpenAiCompatibleConfig, OpenAiCompatibleLlm};
use vpr_provider_openai_transcription::{OpenAiTranscriptionConfig, OpenAiTranscriptionStt};

const PROVIDER_CREDENTIAL_SOURCE_ENV: &str = "VPR_PROVIDER_CREDENTIAL_SOURCE";

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
    /// Builds the Owner Lab provider composition with environment precedence.
    ///
    /// # Errors
    /// Fails for invalid configuration.
    pub fn from_env(require_voice: bool) -> Result<Self, String> {
        let allow_secure_store = credential_source_allows_secure_store()?;
        Self::from_env_with_secure_store(require_voice, allow_secure_store)
    }

    /// Builds providers from process environment only.
    ///
    /// # Errors
    /// Fails for invalid configuration.
    pub fn from_environment(require_voice: bool) -> Result<Self, String> {
        Self::from_env_with_secure_store(require_voice, false)
    }

    fn from_env_with_secure_store(
        require_voice: bool,
        allow_secure_store: bool,
    ) -> Result<Self, String> {
        #[cfg(windows)]
        let profile = if environment_provider_config_complete(require_voice) || !allow_secure_store
        {
            None
        } else {
            load_provider_profile()?
        };
        #[cfg(not(windows))]
        let profile: Option<ProviderCredentialProfile> = {
            let _ = allow_secure_store;
            None
        };
        let (avatar, avatar_descriptor) = build_avatar(profile.as_ref())?;

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
            avatar,
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
    if !avatar_provider_config_complete_with(&mut get) {
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

fn build_avatar(
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
        "did" | "d-id" | "did-agent-streams" => {
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
                DidAgentStreamsConfig::new(endpoint.clone(), api_key, agent_id.clone())
                    .with_fluent(fluent),
            )
            .map_err(|_| "D-ID provider configuration rejected".to_string())?;
            Ok((
                Box::new(provider),
                descriptor(
                    "avatar",
                    "did-agent-streams",
                    "configured-agent",
                    &[
                        &endpoint,
                        &agent_id,
                        if fluent { "fluent" } else { "legacy" },
                    ],
                ),
            ))
        }
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

fn credential_source_allows_secure_store() -> Result<bool, String> {
    credential_source_allows_secure_store_with(
        optional_env(PROVIDER_CREDENTIAL_SOURCE_ENV).as_deref(),
    )
}

fn credential_source_allows_secure_store_with(value: Option<&str>) -> Result<bool, String> {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        None | Some("auto") => Ok(true),
        Some("environment") => Ok(false),
        Some(_) => Err(format!(
            "environment variable {PROVIDER_CREDENTIAL_SOURCE_ENV} must be auto or environment"
        )),
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
