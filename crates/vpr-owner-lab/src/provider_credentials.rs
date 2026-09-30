use serde::{Deserialize, Serialize};

#[cfg(windows)]
const WINDOWS_SERVICE: &str = "Virtual-Persona-Runtime";
#[cfg(windows)]
const WINDOWS_LEGACY_PROFILE_ACCOUNT: &str = "rt0-provider-profile-v1";
#[cfg(windows)]
const WINDOWS_PROFILE_STORE_PREFIX: &str = "rt0-provider-profile-v2";
const PROFILE_SCHEMA: &str = "vpr-rt0-provider-profile-1";

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderCredentialProfile {
    schema_version: String,
    pub did_api_key: String,
    pub did_agent_id: String,
    pub did_endpoint: String,
    pub did_fluent: bool,
    #[serde(default = "default_avatar_provider")]
    pub avatar_provider: String,
    #[serde(default)]
    pub local_avatar_endpoint: Option<String>,
    #[serde(default)]
    pub local_avatar_api_token: Option<String>,
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
        did_api_key: &str,
        did_agent_id: &str,
        deepgram_api_key: String,
        deepseek_api_key: String,
    ) -> Self {
        Self {
            schema_version: PROFILE_SCHEMA.into(),
            did_api_key: normalize_did_api_key(did_api_key),
            did_agent_id: did_agent_id.trim().to_owned(),
            did_endpoint: "https://api.d-id.com".into(),
            did_fluent: false,
            avatar_provider: "did".into(),
            local_avatar_endpoint: None,
            local_avatar_api_token: None,
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

    /// Replaces only D-ID credentials while preserving STT and LLM credentials.
    pub fn replace_did_credentials(&mut self, did_api_key: &str, did_agent_id: &str) {
        self.did_api_key = normalize_did_api_key(did_api_key);
        did_agent_id.trim().clone_into(&mut self.did_agent_id);
        self.did_fluent = false;
        self.avatar_provider = "did".into();
    }

    /// Selects the self-hosted avatar worker while preserving D-ID as a fallback provider.
    ///
    /// # Errors
    /// Returns a redacted configuration error when endpoint or token is empty, or when the
    /// resulting provider profile is incomplete.
    pub fn select_local_avatar(&mut self, endpoint: &str, api_token: &str) -> Result<(), String> {
        let endpoint = endpoint.trim();
        let api_token = api_token.trim();
        if endpoint.is_empty() || api_token.is_empty() {
            return Err("local avatar endpoint/token cannot be empty".into());
        }
        self.avatar_provider = "local-open-source".into();
        self.local_avatar_endpoint = Some(endpoint.to_owned());
        self.local_avatar_api_token = Some(api_token.to_owned());
        self.validate()
    }

    /// Selects the stored D-ID binding without deleting a configured local fallback.
    pub fn select_did_avatar(&mut self) {
        self.avatar_provider = "did".into();
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
            ("avatar provider", self.avatar_provider.as_str()),
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
        match self.avatar_provider.as_str() {
            "did" | "d-id" | "did-agent-streams" => {}
            "local" | "local-open-source" => {
                if self
                    .local_avatar_endpoint
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty())
                    || self
                        .local_avatar_api_token
                        .as_deref()
                        .is_none_or(|value| value.trim().is_empty())
                {
                    return Err(
                        "secure provider profile local avatar configuration is incomplete".into(),
                    );
                }
            }
            _ => return Err("secure provider profile avatar provider is unsupported".into()),
        }
        Ok(())
    }
}

fn default_avatar_provider() -> String {
    "did".into()
}

fn normalize_did_api_key(value: &str) -> String {
    let value = value.trim();
    value
        .strip_prefix("Basic ")
        .or_else(|| value.strip_prefix("basic "))
        .unwrap_or(value)
        .trim()
        .to_owned()
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
    use super::{
        ProviderCredentialProfile, WINDOWS_LEGACY_PROFILE_ACCOUNT, WINDOWS_PROFILE_STORE_PREFIX,
        WINDOWS_SERVICE, normalize_did_api_key,
    };
    use crate::windows_secure_store::ChunkedCredentialStore;

    fn store() -> ChunkedCredentialStore {
        ChunkedCredentialStore::new(
            WINDOWS_SERVICE,
            WINDOWS_PROFILE_STORE_PREFIX,
            Some(WINDOWS_LEGACY_PROFILE_ACCOUNT),
        )
    }

    pub(super) fn load() -> Result<Option<ProviderCredentialProfile>, String> {
        let Some(raw) = store().load()? else {
            return Ok(None);
        };
        let mut profile: ProviderCredentialProfile = serde_json::from_str(&raw)
            .map_err(|_| "Windows Credential Manager contains an invalid VPR provider profile")?;
        profile.did_api_key = normalize_did_api_key(&profile.did_api_key);
        profile.did_agent_id = profile.did_agent_id.trim().to_owned();
        profile.validate()?;
        // The historical working RT0 Windows configuration left VPR_DID_FLUENT unset.
        // Early secure-profile builds accidentally persisted `true`; normalize those profiles
        // at read time so existing credentials keep working without re-entry.
        profile.did_fluent = false;
        Ok(Some(profile))
    }

    pub(super) fn save(profile: &ProviderCredentialProfile) -> Result<(), String> {
        let raw = serde_json::to_string(profile)
            .map_err(|_| "secure provider profile serialization failed")?;
        store()
            .save(&raw)
            .map_err(|_| "Windows Credential Manager write failed".to_owned())
    }

    pub(super) fn delete() -> Result<(), String> {
        store()
            .delete()
            .map_err(|_| "Windows Credential Manager delete failed".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderCredentialProfile;

    #[test]
    fn canonical_rt0_profile_keeps_selected_provider_stack() {
        let profile = ProviderCredentialProfile::canonical_rt0(
            "did-secret",
            "did-agent",
            "deepgram-secret".into(),
            "deepseek-secret".into(),
        );
        assert_eq!(profile.stt_provider, "deepgram");
        assert_eq!(profile.stt_model, "nova-3");
        assert_eq!(profile.llm_provider, "deepseek");
        assert_eq!(profile.llm_model, "deepseek-flash");
        assert!(!profile.did_fluent);
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn did_key_accepts_accidental_basic_prefix_without_persisting_it() {
        let profile = ProviderCredentialProfile::canonical_rt0(
            "Basic user:password",
            " agent-7 ",
            "deepgram-secret".into(),
            "deepseek-secret".into(),
        );
        assert_eq!(profile.did_api_key, "user:password");
        assert_eq!(profile.did_agent_id, "agent-7");
    }

    #[test]
    fn replacing_did_credentials_preserves_voice_provider_secrets() {
        let mut profile = ProviderCredentialProfile::canonical_rt0(
            "old-user:old-password",
            "old-agent",
            "deepgram-secret".into(),
            "deepseek-secret".into(),
        );
        profile.replace_did_credentials("Basic new-user:new-password", "new-agent");
        assert_eq!(profile.did_api_key, "new-user:new-password");
        assert_eq!(profile.did_agent_id, "new-agent");
        assert_eq!(profile.stt_api_key, "deepgram-secret");
        assert_eq!(profile.llm_api_key, "deepseek-secret");
        assert!(!profile.did_fluent);
    }

    #[test]
    fn historical_profile_without_avatar_fields_defaults_to_did() {
        let raw = r#"{
            "schema_version":"vpr-rt0-provider-profile-1",
            "did_api_key":"did-secret",
            "did_agent_id":"did-agent",
            "did_endpoint":"https://api.d-id.com",
            "did_fluent":false,
            "stt_provider":"deepgram",
            "stt_endpoint":"https://api.deepgram.com/v1/listen",
            "stt_api_key":"deepgram-secret",
            "stt_model":"nova-3",
            "llm_provider":"deepseek",
            "llm_endpoint":"https://api.deepseek.com/chat/completions",
            "llm_api_key":"deepseek-secret",
            "llm_model":"deepseek-flash"
        }"#;
        let profile: ProviderCredentialProfile = serde_json::from_str(raw).unwrap();
        assert_eq!(profile.avatar_provider, "did");
        assert!(profile.local_avatar_endpoint.is_none());
        assert!(profile.local_avatar_api_token.is_none());
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn local_avatar_selection_is_persistable_without_deleting_did_fallback() {
        let mut profile = ProviderCredentialProfile::canonical_rt0(
            "did-secret",
            "did-agent",
            "deepgram-secret".into(),
            "deepseek-secret".into(),
        );
        profile
            .select_local_avatar("https://avatar.example.test", "worker-secret")
            .unwrap();
        assert_eq!(profile.avatar_provider, "local-open-source");
        assert_eq!(profile.did_agent_id, "did-agent");
        assert_eq!(
            profile.local_avatar_endpoint.as_deref(),
            Some("https://avatar.example.test")
        );
        assert!(profile.validate().is_ok());
        profile.select_did_avatar();
        assert_eq!(profile.avatar_provider, "did");
    }

    #[test]
    fn empty_secret_fails_closed() {
        let mut profile = ProviderCredentialProfile::canonical_rt0(
            "did-secret",
            "did-agent",
            "deepgram-secret".into(),
            "deepseek-secret".into(),
        );
        profile.llm_api_key.clear();
        assert!(profile.validate().is_err());
    }
}
