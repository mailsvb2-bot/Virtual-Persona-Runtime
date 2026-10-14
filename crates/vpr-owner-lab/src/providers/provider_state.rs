use super::{ProviderBundle, ProviderDescriptor};
use vpr_evaluation::{
    ProviderRole, ProviderStateBinding, ProviderStateManifest, RT0_PROVIDER_STATE_SCHEMA,
};

impl ProviderBundle {
    /// Returns the sanitized provider-state manifest for the exact provider composition.
    ///
    /// The manifest contains no credentials. Voice evidence requires all RT0 provider roles.
    ///
    /// # Errors
    /// Fails when STT or LLM configuration is absent.
    pub fn provider_state_manifest(&self) -> Result<ProviderStateManifest, String> {
        let stt = self
            .stt_descriptor
            .as_ref()
            .ok_or_else(|| "provider state requires STT configuration".to_owned())?;
        let llm = self
            .llm_descriptor
            .as_ref()
            .ok_or_else(|| "provider state requires LLM configuration".to_owned())?;
        let mut providers = vec![
                binding(ProviderRole::Stt, stt),
                binding(ProviderRole::Llm, llm),
                binding(ProviderRole::Avatar, &self.avatar_descriptor),
            ];
        if let Some(tts) = &self.tts_descriptor {
            let mut active = binding(ProviderRole::Tts, tts);
            if let Some(profile) = &self.voice_profile {
                active.voice_profile_status = Some(match profile.status {
                    crate::VoiceProfileStatus::Interim => "INTERIM",
                    crate::VoiceProfileStatus::Candidate => "CANDIDATE",
                    crate::VoiceProfileStatus::Accepted => "ACCEPTED",
                }.into());
                active.profile_id = Some(profile.profile_id.clone());
                active.version = Some(profile.version);
            }
            providers.push(active);
        }
        Ok(ProviderStateManifest {
            schema_version: RT0_PROVIDER_STATE_SCHEMA.into(),
            providers,
        })
    }
}

fn binding(role: ProviderRole, descriptor: &ProviderDescriptor) -> ProviderStateBinding {
    ProviderStateBinding {
        role,
        provider: descriptor.provider.clone(),
        model_or_representation: descriptor.model_or_representation.clone(),
        configuration_fingerprint_sha256: descriptor.configuration_fingerprint_sha256.clone(),
        voice_profile_status: None,
        profile_id: None,
        version: None,
    }
}
