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
        Ok(ProviderStateManifest {
            schema_version: RT0_PROVIDER_STATE_SCHEMA.into(),
            providers: vec![
                binding(ProviderRole::Stt, stt),
                binding(ProviderRole::Llm, llm),
                binding(ProviderRole::Avatar, &self.avatar_descriptor),
            ],
        })
    }
}

fn binding(role: ProviderRole, descriptor: &ProviderDescriptor) -> ProviderStateBinding {
    ProviderStateBinding {
        role,
        provider: descriptor.provider.clone(),
        model_or_representation: descriptor.model_or_representation.clone(),
        configuration_fingerprint_sha256: descriptor.configuration_fingerprint_sha256.clone(),
    }
}
