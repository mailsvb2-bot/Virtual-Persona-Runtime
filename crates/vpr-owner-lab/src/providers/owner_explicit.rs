//! Explicit-owner provider composition. This is the same canonical bundle
//! and adapter factory as legacy Owner Lab, but without global credentials.
use super::{
    ProviderBundle, ProviderCredentialProfile, RealtimeAvatarPort,
    build_llm, build_selected_avatar, build_stt, ensure_avatar_pcm_compatible,
    required_value,
};
use super::tts_factory::build_tts_explicit;

#[derive(Clone, Copy)]
pub(super) enum ProviderSource {
    EnvironmentFirst,
    ExplicitOnly,
}

impl ProviderBundle {
    /// Build the existing canonical adapters using only one explicitly selected
    /// owner's credential profile. STT/LLM/TTS process environment and the
    /// legacy Windows secure profile must never override that owner.
    ///
    /// The supplied avatar must already be selected for this owner.
    ///
    /// # Errors
    /// Rejects invalid credentials, missing TTS, or incompatible private PCM.
    pub fn from_explicit_owner_profile_with_avatar(
        profile: &ProviderCredentialProfile,
        avatar: Box<dyn RealtimeAvatarPort>,
    ) -> Result<Self, String> {
        profile.validate()?;
        let (avatar, avatar_descriptor) = build_selected_avatar(Some(avatar), None)?;
        let (stt, stt_descriptor) = build_stt(
            &profile.stt_provider.to_ascii_lowercase(),
            Some(profile),
            ProviderSource::ExplicitOnly,
        )?;
        let (llm, llm_descriptor) = build_llm(
            &profile.llm_provider.to_ascii_lowercase(),
            Some(profile),
            ProviderSource::ExplicitOnly,
        )?;
        let configured_tts = profile.tts.as_ref().ok_or("OWNER_TTS_NOT_CONFIGURED")?;
        configured_tts.validate()?;
        ensure_avatar_pcm_compatible(avatar.as_ref())?;
        let (tts, tts_descriptor) = build_tts_explicit(configured_tts)?;
        Ok(Self {
            avatar,
            stt: Some(stt),
            llm: Some(llm),
            tts: Some(tts),
            tts_name: Some(configured_tts.provider.clone()),
            tts_fallback: None,
            tts_fallback_name: None,
            avatar_descriptor,
            stt_descriptor: Some(stt_descriptor),
            llm_descriptor: Some(llm_descriptor),
            tts_descriptor: Some(tts_descriptor),
            tts_fallback_descriptor: None,
            voice_profile: None,
        })
    }

}
pub(super) fn provider_value(
    name: &'static str,
    stored: Option<&str>,
    source: ProviderSource,
) -> Result<String, String> {
    match source {
        ProviderSource::EnvironmentFirst => required_value(name, stored),
        ProviderSource::ExplicitOnly => stored
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
            .ok_or_else(|| format!("owner provider setting {name} missing")),
    }
}

