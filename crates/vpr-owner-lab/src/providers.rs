use std::env;

mod avatar_factory;
mod avatar_selection;
mod provider_state;
mod tts_factory;
use avatar_factory::build_avatar;
#[cfg(any(windows, test))]
use avatar_selection::avatar_provider_config_complete_with;
use avatar_selection::select_environment_avatar_provider_with;
use sha2::{Digest, Sha256};
use tts_factory::{build_tts, build_tts_fallback, ensure_avatar_pcm_compatible};

use crate::provider_credentials::ProviderCredentialProfile;
#[cfg(windows)]
use crate::provider_credentials::load_provider_profile;
use crate::voice_profile::VoiceProfile;
use vpr_integration::{LlmPort, RealtimeAvatarPort, SttPort, TtsPort};
use vpr_provider_anthropic::{AnthropicConfig, AnthropicLlm};
use vpr_provider_deepgram_stt::{DeepgramStt, DeepgramSttConfig};
use vpr_provider_gemini::{GeminiConfig, GeminiLlm};
use vpr_provider_openai_compatible::{AuthScheme, OpenAiCompatibleConfig, OpenAiCompatibleLlm};
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
    pub tts: Option<Box<dyn TtsPort>>,
    pub tts_name: Option<String>,
    pub tts_fallback: Option<Box<dyn TtsPort>>,
    pub tts_fallback_name: Option<String>,
    pub avatar_descriptor: ProviderDescriptor,
    pub stt_descriptor: Option<ProviderDescriptor>,
    pub llm_descriptor: Option<ProviderDescriptor>,
    pub tts_descriptor: Option<ProviderDescriptor>,
    pub tts_fallback_descriptor: Option<ProviderDescriptor>,
    pub voice_profile: Option<VoiceProfile>,
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
        let profile = resolve_provider_credential_profile(require_voice, allow_secure_store)?;
        #[cfg(not(windows))]
        let profile: Option<ProviderCredentialProfile> = {
            let _ = (require_voice, allow_secure_store);
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
        let voice_profile = VoiceProfile::from_env()?;
        validate_voice_profile_configuration(voice_profile.as_ref(), profile.as_ref())?;
        let strict = env::var("VPR_OWNER_LAB_RT0_EVIDENCE")
            .ok()
            .is_some_and(|value| value.eq_ignore_ascii_case("true"));
        let tts_name = optional_env_lower("VPR_OWNER_LAB_TTS_PROVIDER").or_else(|| {
            profile
                .as_ref()?
                .tts
                .as_ref()
                .map(|tts| tts.provider.to_ascii_lowercase())
        });
        if voice_profile.is_some() && tts_name.is_none() {
            return Err("VOICE_PROFILE_TTS_NOT_CONFIGURED".into());
        }
        if strict {
            validate_strict_voice_selection(
                tts_name.as_deref(),
                voice_profile.as_ref(),
                optional_env_lower("VPR_OWNER_LAB_TTS_ALT_PROVIDER").is_some(),
            )?;
        }
        if tts_name.is_some() {
            ensure_avatar_pcm_compatible(avatar.as_ref())?;
        }
        let tts = tts_name
            .as_deref()
            .map(|name| {
                build_tts(
                    name,
                    profile.as_ref().and_then(|profile| profile.tts.as_ref()),
                )
            })
            .transpose()?;
        let (tts_fallback_name, tts_fallback) = configure_tts_fallback(tts_name.as_deref())?;
        let (tts, tts_descriptor) = match tts {
            Some((port, mut descriptor)) => {
                if let Some(voice) = &voice_profile {
                    descriptor = bind_tts_descriptor(descriptor, voice)?;
                }
                (Some(port), Some(descriptor))
            }
            None => (None, None),
        };
        let (tts_fallback, tts_fallback_descriptor) = match tts_fallback {
            Some((port, descriptor)) => (Some(port), Some(descriptor)),
            None => (None, None),
        };
        Ok(Self {
            avatar,
            stt,
            llm,
            tts,
            tts_name,
            tts_fallback,
            tts_fallback_name,
            avatar_descriptor,
            stt_descriptor,
            llm_descriptor,
            tts_descriptor,
            tts_fallback_descriptor,
            voice_profile,
        })
    }
}
#[cfg(windows)]
fn resolve_provider_credential_profile(
    require_voice: bool,
    allow_secure_store: bool,
) -> Result<Option<ProviderCredentialProfile>, String> {
    if !allow_secure_store
        || (environment_provider_config_complete(require_voice)
            && optional_env_lower("VPR_OWNER_LAB_TTS_PROVIDER").is_some())
    {
        Ok(None)
    } else {
        load_provider_profile()
    }
}

type ConfiguredTts = (Box<dyn TtsPort>, ProviderDescriptor);
type TtsFallbackConfiguration = (Option<String>, Option<ConfiguredTts>);

fn configure_tts_fallback(primary: Option<&str>) -> Result<TtsFallbackConfiguration, String> {
    let fallback = optional_env_lower("VPR_OWNER_LAB_TTS_ALT_PROVIDER");
    if fallback.is_some() && primary.is_none() {
        return Err("TTS fallback requires a primary canonical TTS provider".into());
    }
    if fallback.as_deref() == primary && fallback.is_some() {
        return Err("TTS fallback must use a distinct provider identifier".into());
    }
    let configured = fallback.as_deref().map(build_tts_fallback).transpose()?;
    Ok((fallback, configured))
}

fn validate_voice_profile_configuration(
    voice_profile: Option<&VoiceProfile>,
    credentials: Option<&ProviderCredentialProfile>,
) -> Result<(), String> {
    let Some(voice) = voice_profile else {
        return Ok(());
    };
    for (name, expected) in [
        ("VPR_OWNER_LAB_TTS_PROVIDER", voice.provider.as_str()),
        ("VPR_OWNER_LAB_TTS_MODEL", voice.model.as_str()),
        ("VPR_OWNER_LAB_TTS_VOICE", voice.voice.as_str()),
    ] {
        let effective = env::var(name).ok().or_else(|| {
            let tts = credentials?.tts.as_ref()?;
            match name {
                "VPR_OWNER_LAB_TTS_PROVIDER" => Some(tts.provider.clone()),
                "VPR_OWNER_LAB_TTS_MODEL" => Some(tts.model.clone()),
                "VPR_OWNER_LAB_TTS_VOICE" => Some(tts.voice.clone()),
                _ => None,
            }
        });
        if effective.as_deref() != Some(expected) {
            return Err(format!("VOICE_PROFILE_ENV_CONFLICT: {name}"));
        }
    }
    Ok(())
}

fn validate_strict_voice_selection(
    primary_tts: Option<&str>,
    profile: Option<&VoiceProfile>,
    fallback_configured: bool,
) -> Result<(), String> {
    if fallback_configured {
        return Err("RT0_EVIDENCE_TTS_FALLBACK_FORBIDDEN".into());
    }
    if primary_tts.is_some() && profile.is_none() {
        return Err("RT0_EVIDENCE_VOICE_PROFILE_REQUIRED".into());
    }
    if let Some(profile) = profile {
        profile.validate_strict()?;
    }
    Ok(())
}

/// Binds exact nonsecret voice identity to the TTS configuration fingerprint.
/// Any provider, model, speaker, status, rights digest or version change must
/// invalidate all earlier RT0 evidence.
fn bind_tts_descriptor(
    mut descriptor: ProviderDescriptor,
    profile: &VoiceProfile,
) -> Result<ProviderDescriptor, String> {
    descriptor.model_or_representation = format!(
        "{}#{}@{}:v{}",
        profile.model, profile.voice, profile.profile_id, profile.version,
    );
    let bytes = serde_json::to_vec(profile).map_err(|_| "INVALID_VOICE_PROFILE")?;
    let mut hasher = Sha256::new();
    hasher.update(descriptor.configuration_fingerprint_sha256.as_bytes());
    hasher.update(&bytes);
    descriptor.configuration_fingerprint_sha256 = format!("{:x}", hasher.finalize());
    Ok(descriptor)
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
            config = config.with_max_tokens(96);
            if canonical == "deepseek" {
                config = config.with_reasoning_effort("none").with_thinking_disabled();
            } else if canonical == "yandex" {
                if endpoint != "https://ai.api.cloud.yandex.net/v1/chat/completions" {
                    return Err("YANDEX_LLM_ENDPOINT_MISMATCH".into());
                }
                let folder = required_value("VPR_OWNER_LAB_LLM_PROJECT_ID", None)?;
                if !model.starts_with(&format!("gpt://{folder}/")) {
                    return Err("YANDEX_LLM_MODEL_FOLDER_MISMATCH".into());
                }
                config = config.with_auth_scheme(AuthScheme::ApiKey)
                    .with_stream_usage(false)
                    .with_project_id(&folder)
                    .map_err(|_| "YANDEX_LLM_PROJECT_ID_INVALID")?;
            } else if canonical == "proxyapi-deepseek" {
                if endpoint != "https://api.proxyapi.ru/v1/chat/completions" {
                    return Err("PROXYAPI_LLM_ENDPOINT_MISMATCH".into());
                }
                // No DeepSeek-specific controls without a verified ProxyAPI wire contract.
                config = config.with_stream_usage(false);
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
    let realtime_profile = match canonical {
        "deepseek" => "thinking=disabled;reasoning=none;max_tokens=96;stream_usage=on;auth=bearer",
        "yandex" => "max_tokens=96;stream_usage=off;auth=apikey;project=set",
        "proxyapi-deepseek" => "max_tokens=96;stream_usage=off;auth=bearer",
        "openai-compatible" => "max_tokens=96;stream_usage=on;auth=bearer",
        _ => "provider-default",
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
        "yandex" => Some("yandex"),
        "proxyapi-deepseek" => Some("proxyapi-deepseek"),
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
mod tests;
