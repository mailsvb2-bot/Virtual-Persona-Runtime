//! Non-secret, versioned voice selection. Identity stays provider-neutral.
use std::{env, fs, path::Path};
use serde::{Deserialize, Serialize};

pub const VOICE_PROFILE_SCHEMA: &str = "vpr-voice-profile-0.1";
const MAX_PROFILE_BYTES: u64 = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VoiceProfileStatus { Interim, Candidate, Accepted }

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceProfile {
    pub schema_version: String,
    pub profile_id: String,
    pub version: u32,
    pub status: VoiceProfileStatus,
    pub provider: String,
    pub model: String,
    pub voice: String,
    pub rights_ref_sha256: Option<String>,
}

impl VoiceProfile {
    /// Parse a bounded, secret-free voice profile and validate its identity.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_PROFILE_BYTES as usize {
            return Err("VOICE_PROFILE_TOO_LARGE".into());
        }
        let profile: Self = serde_json::from_slice(bytes)
            .map_err(|_| "INVALID_VOICE_PROFILE_JSON")?;
        if profile.schema_version != VOICE_PROFILE_SCHEMA
            || !(3..=64).contains(&profile.profile_id.len())
            || !profile.profile_id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || profile.version == 0
            || [&profile.provider, &profile.model, &profile.voice].iter().any(|v| v.is_empty() || v.trim() != v.as_str() || v.len() > 128)
        {
            return Err("INVALID_VOICE_PROFILE_IDENTITY".into());
        }
        if profile.provider == "metrotrance-qwen-preset"
            && !matches!(profile.voice.as_str(),
                "Ryan" | "Aiden" | "Vivian" | "Serena" | "Uncle_Fu" |
                "Dylan" | "Eric" | "Ono_Anna" | "Sohee")
        {
            return Err("UNSUPPORTED_QWEN_PRESET_VOICE".into());
        }
        if profile.rights_ref_sha256.as_deref().is_some_and(|digest| !is_digest(digest))
            || profile.status == VoiceProfileStatus::Accepted && profile.rights_ref_sha256.is_none()
        {
            return Err("VOICE_RIGHTS_REQUIRED_OR_INVALID".into());
        }
        Ok(profile)
    }

    /// Reading from a local path is never an implicit provider egress.
    pub fn load(path: &Path) -> Result<Self, String> {
        let size = fs::metadata(path).map_err(|_| "VOICE_PROFILE_UNAVAILABLE")?.len();
        if size > MAX_PROFILE_BYTES {
            return Err("VOICE_PROFILE_TOO_LARGE".into());
        }
        let bytes = fs::read(path).map_err(|_| "VOICE_PROFILE_UNAVAILABLE")?;
        Self::parse(&bytes)
    }

    pub fn from_env() -> Result<Option<Self>, String> {
        match env::var("VPR_OWNER_LAB_VOICE_PROFILE_PATH") {
            Ok(path) if !path.trim().is_empty() => Self::load(Path::new(&path)).map(Some),
            Ok(_) => Err("VOICE_PROFILE_PATH_BLANK".into()),
            Err(env::VarError::NotPresent) => Ok(None),
            Err(env::VarError::NotUnicode(_)) => Err("VOICE_PROFILE_PATH_INVALID".into()),
        }
    }

    pub fn validate_strict(&self) -> Result<(), String> {
        if self.status == VoiceProfileStatus::Candidate {
            return Err("CANDIDATE_VOICE_NOT_ALLOWED_FOR_RT0_EVIDENCE".into());
        }
        Ok(())
    }
}

fn is_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn example() -> VoiceProfile {
        VoiceProfile { schema_version: VOICE_PROFILE_SCHEMA.into(), profile_id: "interim-qwen3-serena".into(),
            version: 1, status: VoiceProfileStatus::Interim, provider: "metrotrance-qwen-preset".into(),
            model: "metrotrance-qwen-preset".into(), voice: "Serena".into(), rights_ref_sha256: None }
    }
    #[test]
    fn validates_interim_and_rejects_unrecognized_secret_fields() {
        let bytes = serde_json::to_vec(&example()).unwrap();
        assert_eq!(VoiceProfile::parse(&bytes).unwrap().status, VoiceProfileStatus::Interim);
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["api_key"] = serde_json::json!("should-not-be-in-profile");
        assert!(VoiceProfile::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    #[test]
    fn accepted_requires_valid_rights_digest_and_candidate_fails_strict() {
        let mut profile = example();
        profile.status = VoiceProfileStatus::Accepted;
        assert!(VoiceProfile::parse(&serde_json::to_vec(&profile).unwrap()).is_err());
        profile.rights_ref_sha256 = Some("a".repeat(64));
        assert!(VoiceProfile::parse(&serde_json::to_vec(&profile).unwrap()).is_ok());
        profile.status = VoiceProfileStatus::Candidate;
        assert!(profile.validate_strict().is_err());
    }
    #[test]
    fn rejects_unbound_local_preset_speaker() {
        let mut profile = example();
        profile.voice = "local".into();
        assert!(VoiceProfile::parse(&serde_json::to_vec(&profile).unwrap()).is_err());
    }

    #[test]
    fn rejects_invalid_identity_or_empty_and_oversized_input() {
        let mut profile = example();
        profile.version = 0;
        assert!(VoiceProfile::parse(&serde_json::to_vec(&profile).unwrap()).is_err());
        assert!(VoiceProfile::parse(&vec![b' '; 16_385]).is_err());
        assert!(VoiceProfile::parse(b"{}").is_err());
    }
}
