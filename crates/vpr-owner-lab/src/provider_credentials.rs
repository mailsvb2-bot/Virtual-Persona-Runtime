use serde::{Deserialize, Serialize};

#[cfg(windows)]
const WINDOWS_SERVICE: &str = "Virtual-Persona-Runtime";
#[cfg(windows)]
const WINDOWS_PROFILE_ACCOUNT: &str = "rt0-provider-profile-v1";
const PROFILE_SCHEMA: &str = "vpr-rt0-provider-profile-1";

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderCredentialProfile {
    schema_version: String,
    pub did_api_key: String,
    pub did_agent_id: String,
    pub did_endpoint: String,
    pub did_fluent: bool,
    pub stt_provider: String,
    pub stt_endpoint: String,
    pub stt_api_key: String,
    pub stt_model: String,
    pub llm_provider: String,
    pub llm_endpoint: String,
    pub llm_api_key: String,
    pub llm_model: String,
}

impl ProviderCredentialProfile {
    #[must_use]
    pub fn canonical_rt0(
        did_api_key: String,
        did_agent_id: String,
        deepgram_api_key: String,
        deepseek_api_key: String,
    ) -> Self {
        Self {
            schema_version: PROFILE_SCHEMA.into(),
            did_api_key,
            did_agent_id,
            did_endpoint: "https://api.d-id.com".into(),
            did_fluent: true,
            stt_provider: "deepgram".into(),
            stt_endpoint: "https://api.deepgram.com/v1/listen".into(),
            stt_api_key: deepgram_api_key,
            stt_model: "nova-3".into(),
            llm_provider: "deepseek".into(),
            llm_endpoint: "https://api.deepseek.com/chat/completions".into(),
            llm_api_key: deepseek_api_key,
            llm_model: "deepseek-flash".into(),
        }
    }

    /// Validates that the stored profile is complete and has the expected schema.
    ///
    /// # Errors
    /// Returns a redacted error when the schema is unsupported or a required field is empty.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != PROFILE_SCHEMA {
            return Err("secure provider profile schema is unsupported".into());
        }
        for (label, value) in [
            ("D-ID API key", self.did_api_key.as_str()),
            ("D-ID agent ID", self.did_agent_id.as_str()),
            ("STT provider", self.stt_provider.as_str()),
            ("STT endpoint", self.stt_endpoint.as_str()),
            ("STT API key", self.stt_api_key.as_str()),
            ("STT model", self.stt_model.as_str()),
            ("LLM provider", self.llm_provider.as_str()),
            ("LLM endpoint", self.llm_endpoint.as_str()),
            ("LLM API key", self.llm_api_key.as_str()),
            ("LLM model", self.llm_model.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(format!("secure provider profile field {label} is empty"));
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
/// Loads the current Windows user's VPR provider profile from Windows Credential Manager.
///
/// # Errors
/// Returns a redacted error when the credential store cannot be read or the stored profile is invalid.
pub fn load_provider_profile() -> Result<Option<ProviderCredentialProfile>, String> {
    platform::load()
}

#[cfg(windows)]
/// Persists a validated VPR provider profile in Windows Credential Manager.
///
/// # Errors
/// Returns a redacted error when validation, serialization, or credential-store persistence fails.
pub fn save_provider_profile(profile: &ProviderCredentialProfile) -> Result<(), String> {
    profile.validate()?;
    platform::save(profile)
}

#[cfg(windows)]
/// Deletes the current Windows user's persisted VPR provider profile.
///
/// # Errors
/// Returns a redacted error when Windows Credential Manager cannot delete the credential.
pub fn delete_provider_profile() -> Result<(), String> {
    platform::delete()
}

#[cfg(windows)]
mod platform {
    use super::{ProviderCredentialProfile, WINDOWS_PROFILE_ACCOUNT, WINDOWS_SERVICE};
    use keyring::{Entry, Error as KeyringError};

    fn entry() -> Result<Entry, String> {
        Entry::new(WINDOWS_SERVICE, WINDOWS_PROFILE_ACCOUNT)
            .map_err(|_| "Windows Credential Manager entry initialization failed".into())
    }

    pub(super) fn load() -> Result<Option<ProviderCredentialProfile>, String> {
        let raw = match entry()?.get_password() {
            Ok(raw) => raw,
            Err(KeyringError::NoEntry) => return Ok(None),
            Err(_) => return Err("Windows Credential Manager read failed".into()),
        };
        let profile: ProviderCredentialProfile = serde_json::from_str(&raw)
            .map_err(|_| "Windows Credential Manager contains an invalid VPR provider profile")?;
        profile.validate()?;
        Ok(Some(profile))
    }

    pub(super) fn save(profile: &ProviderCredentialProfile) -> Result<(), String> {
        let raw = serde_json::to_string(profile)
            .map_err(|_| "secure provider profile serialization failed")?;
        entry()?
            .set_password(&raw)
            .map_err(|_| "Windows Credential Manager write failed")
    }

    pub(super) fn delete() -> Result<(), String> {
        match entry()?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(_) => Err("Windows Credential Manager delete failed".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderCredentialProfile;

    #[test]
    fn canonical_rt0_profile_keeps_selected_provider_stack() {
        let profile = ProviderCredentialProfile::canonical_rt0(
            "did-secret".into(),
            "did-agent".into(),
            "deepgram-secret".into(),
            "deepseek-secret".into(),
        );
        assert_eq!(profile.stt_provider, "deepgram");
        assert_eq!(profile.stt_model, "nova-3");
        assert_eq!(profile.llm_provider, "deepseek");
        assert_eq!(profile.llm_model, "deepseek-flash");
        assert!(profile.did_fluent);
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn empty_secret_fails_closed() {
        let mut profile = ProviderCredentialProfile::canonical_rt0(
            "did-secret".into(),
            "did-agent".into(),
            "deepgram-secret".into(),
            "deepseek-secret".into(),
        );
        profile.llm_api_key.clear();
        assert!(profile.validate().is_err());
    }
}
