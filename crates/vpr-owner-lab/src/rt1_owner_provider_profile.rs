//! Operator-provisioned credentials for one canonical owner. Secrets are never
//! persisted in the Persona envelope or returned from owner/visitor HTTP APIs.
//! The legacy environment provider path is only permitted for the primary owner.
use serde::{Deserialize, Serialize};
use std::env;

use crate::ProviderCredentialProfile;

const PROFILE_DIR: &str = "VPR_RT1_OWNER_PROVIDER_DIR";
const PROFILE_SCHEMA: &str = "vpr-rt1-owner-provider-profile-1";
#[cfg(windows)]
const WINDOWS_SERVICE: &str = "Virtual-Persona-Runtime";
#[cfg(windows)]
const WINDOWS_OWNER_PREFIX: &str = "rt1-owner-provider-v1";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerProviderProfile {
    schema_version: String,
    owner_id: String,
    pub profile: ProviderCredentialProfile,
    pub worker_endpoint: String,
    pub worker_token: String,
}

impl OwnerProviderProfile {
    /// Construct one owner-specific configuration with a reviewed canonical
    /// owner ID. This object is operator-provisioned, never browser-generated.
    ///
    /// # Errors
    /// Rejects invalid owner ID or incomplete/unsupported voice credentials.
    pub fn new(
        owner_id: &str,
        profile: ProviderCredentialProfile,
        worker_endpoint: String,
        worker_token: String,
    ) -> Result<Self, String> {
        validate_owner_id(owner_id)?;
        let configured = Self {
            schema_version: PROFILE_SCHEMA.into(),
            owner_id: owner_id.to_owned(),
            profile,
            worker_endpoint,
            worker_token,
        };
        configured.validate(owner_id)?;
        Ok(configured)
    }

    /// Verify profile schema and exact owner binding on every platform.
    ///
    /// # Errors
    /// Rejects mismatched owner identity and incomplete or invalid credentials.
    pub fn validate(&self, owner_id: &str) -> Result<(), String> {
        if self.schema_version != PROFILE_SCHEMA
            || self.owner_id != owner_id
            || self.worker_endpoint.trim().is_empty()
            || self.worker_token.trim().is_empty()
            || self.worker_token.chars().any(char::is_control)
            || self.worker_token.trim() != self.worker_token
        {
            return Err("RT1_OWNER_PROVIDER_PROFILE_INVALID".into());
        }
        self.profile
            .validate()
            .map_err(|_| "RT1_OWNER_PROVIDER_PROFILE_INVALID".to_owned())
    }
}

fn validate_owner_id(owner_id: &str) -> Result<(), String> {
    if owner_id.len() != 32 || !owner_id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("RT1_OWNER_PROVIDER_ID_INVALID".into());
    }
    Ok(())
}

#[cfg(windows)]
fn windows_store(owner_id: &str) -> crate::windows_secure_store::ChunkedCredentialStore {
    crate::windows_secure_store::ChunkedCredentialStore::for_owner(
        WINDOWS_SERVICE,
        format!("{WINDOWS_OWNER_PREFIX}-{owner_id}"),
    )
}

/// Store an already validated owner-specific profile under its exact
/// credential-manager namespace. Credentials never enter the Persona record.
/// Use only from trusted operator setup, not public HTTP.
///
/// # Errors
/// Rejects invalid owner identity, incorrect profile binding or a failed
/// Windows Credential Manager write.
#[cfg(windows)]
pub fn save_owner_provider_profile(
    owner_id: &str,
    profile: &OwnerProviderProfile,
) -> Result<(), String> {
    validate_owner_id(owner_id)?;
    profile.validate(owner_id)?;
    let raw = serde_json::to_string(profile).map_err(|_| "RT1_OWNER_PROVIDER_PROFILE_INVALID")?;
    windows_store(owner_id)
        .save(&raw)
        .map_err(|_| "RT1_OWNER_PROVIDER_SECURE_STORE_WRITE_FAILED".into())
}

/// Revoke a single owner's private credentials without touching other owners
/// or the legacy RT0 credential profile.
///
/// # Errors
/// Rejects invalid owner identity or an unavailable secure-store backend.
#[cfg(windows)]
pub fn delete_owner_provider_profile(owner_id: &str) -> Result<(), String> {
    validate_owner_id(owner_id)?;
    windows_store(owner_id)
        .delete()
        .map_err(|_| "RT1_OWNER_PROVIDER_SECURE_STORE_DELETE_FAILED".into())
}

#[cfg(windows)]
fn load_windows_owner_profile(owner_id: &str) -> Result<Option<OwnerProviderProfile>, String> {
    let raw = windows_store(owner_id)
        .load()
        .map_err(|_| "RT1_OWNER_PROVIDER_SECURE_STORE_READ_FAILED")?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let profile: OwnerProviderProfile =
        serde_json::from_str(&raw).map_err(|_| "RT1_OWNER_PROVIDER_PROFILE_INVALID")?;
    profile.validate(owner_id)?;
    Ok(Some(profile))
}

/// Read the operator-owned profile for the canonical owner identity, never
/// a browser-supplied file path. If a profile directory is configured,
/// missing or unsafe per-owner records are fatal: there is no env fallback.
///
/// # Errors
/// Denies invalid owner IDs, insecure/missing credentials and unsupported
/// operating systems. Does not return secret values in error strings.
pub(crate) fn profile_for_owner(owner_id: &str) -> Result<Option<OwnerProviderProfile>, String> {
    validate_owner_id(owner_id)?;
    let Some(path) = env::var_os(PROFILE_DIR) else {
        #[cfg(windows)]
        {
            return load_windows_owner_profile(owner_id);
        }
        #[cfg(not(windows))]
        {
            return Ok(None);
        }
    };
    if path.is_empty() {
        return Err("RT1_OWNER_PROVIDER_DIRECTORY_INVALID".into());
    }
    #[cfg(unix)]
    {
        load_private_profile(std::path::Path::new(&path), owner_id).map(Some)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, owner_id);
        Err("RT1_OWNER_PROVIDER_SECURE_STORE_UNAVAILABLE".into())
    }
}

#[cfg(unix)]
fn load_private_profile(
    dir: &std::path::Path,
    owner_id: &str,
) -> Result<OwnerProviderProfile, String> {
    use std::fs::{self, File};
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let dir_meta =
        fs::symlink_metadata(dir).map_err(|_| "RT1_OWNER_PROVIDER_DIRECTORY_UNAVAILABLE")?;
    if !dir_meta.is_dir()
        || dir_meta.file_type().is_symlink()
        || dir_meta.permissions().mode() & 0o077 != 0
    {
        return Err("RT1_OWNER_PROVIDER_DIRECTORY_INSECURE".into());
    }
    let file_path = dir.join(format!("{owner_id}.json"));
    let initial =
        fs::symlink_metadata(&file_path).map_err(|_| "RT1_OWNER_PROVIDER_PROFILE_UNAVAILABLE")?;
    if !initial.is_file()
        || initial.file_type().is_symlink()
        || initial.permissions().mode() & 0o077 != 0
        || initial.uid() != dir_meta.uid()
        || initial.nlink() != 1
    {
        return Err("RT1_OWNER_PROVIDER_PROFILE_INSECURE".into());
    }
    let file = File::open(&file_path).map_err(|_| "RT1_OWNER_PROVIDER_PROFILE_UNAVAILABLE")?;
    let opened = file
        .metadata()
        .map_err(|_| "RT1_OWNER_PROVIDER_PROFILE_UNAVAILABLE")?;
    if initial.dev() != opened.dev()
        || initial.ino() != opened.ino()
        || opened.permissions().mode() & 0o077 != 0
    {
        return Err("RT1_OWNER_PROVIDER_PROFILE_CHANGED".into());
    }
    let mut raw = String::new();
    file.take(32_769)
        .read_to_string(&mut raw)
        .map_err(|_| "RT1_OWNER_PROVIDER_PROFILE_INVALID")?;
    if raw.len() > 32_768 {
        return Err("RT1_OWNER_PROVIDER_PROFILE_TOO_LARGE".into());
    }
    let profile: OwnerProviderProfile =
        serde_json::from_str(&raw).map_err(|_| "RT1_OWNER_PROVIDER_PROFILE_INVALID")?;
    profile.validate(owner_id)?;
    Ok(profile)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn private_owner_profiles_are_isolated_and_fail_closed() {
        let mut nonce = [0_u8; 8];
        getrandom::fill(&mut nonce).unwrap();
        let unique = u64::from_be_bytes(nonce);
        let dir = env::temp_dir().join(format!(
            "vpr-owner-provider-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();

        let a = "a".repeat(32);
        let b = "b".repeat(32);
        let mut profile = ProviderCredentialProfile::canonical_rt0(
            "avatar-token",
            "agent-a",
            "stt-token".into(),
            "llm-token".into(),
        );
        profile
            .set_tts(crate::StoredTtsCredential {
                provider: "openai-speech".into(),
                endpoint: "https://api.openai.com/v1/audio/speech".into(),
                api_key: "tts-token".into(),
                model: "tts-1".into(),
                voice: "alloy".into(),
            })
            .unwrap();
        let raw = serde_json::json!({
            "schema_version": PROFILE_SCHEMA,
            "owner_id": a,
            "profile": profile,
            "worker_endpoint": "https://worker.example.test",
            "worker_token": "worker-owner-a"
        })
        .to_string();
        let path_a = dir.join(format!("{a}.json"));
        fs::write(&path_a, raw).unwrap();
        fs::set_permissions(&path_a, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(load_private_profile(&dir, &a).unwrap().owner_id, a);
        assert!(load_private_profile(&dir, &b).is_err());

        let path_b = dir.join(format!("{b}.json"));
        symlink(&path_a, &path_b).unwrap();
        assert!(load_private_profile(&dir, &b).is_err());
        fs::remove_file(&path_b).unwrap();

        fs::set_permissions(&path_a, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_private_profile(&dir, &a).is_err());
        fs::set_permissions(&path_a, fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(load_private_profile(&dir, &a).is_err());
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_file(path_a).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}

#[cfg(all(test, windows))]
mod windows_owner_tests {
    use super::*;

    #[test]
    fn windows_owner_profiles_are_independent_and_legacy_store_is_unchanged() {
        let mut random = [0_u8; 32];
        getrandom::fill(&mut random).unwrap();
        let a = format!("{:032x}", u128::from_be_bytes(random[..16].try_into().unwrap()));
        let b = format!("{:032x}", u128::from_be_bytes(random[16..].try_into().unwrap()));
        let make_profile = |owner_id: &str, key: &str| {
            let mut voice = ProviderCredentialProfile::local_rt0(
                "https://avatar.example.test",
                key,
                format!("stt-{key}"),
                format!("llm-{key}"),
            );
            voice
                .set_tts(crate::StoredTtsCredential {
                    provider: "openai-speech".into(),
                    endpoint: "https://api.openai.com/v1/audio/speech".into(),
                    api_key: format!("tts-{key}"),
                    model: "tts-1".into(),
                    voice: "alloy".into(),
                })
                .unwrap();
            OwnerProviderProfile::new(
                owner_id,
                voice,
                "https://worker.example.test".into(),
                format!("worker-{key}"),
            )
            .unwrap()
        };
        let first = make_profile(&a, "owner-a");
        let second = make_profile(&b, "owner-b");
        let _ = delete_owner_provider_profile(&a);
        let _ = delete_owner_provider_profile(&b);
        save_owner_provider_profile(&a, &first).unwrap();
        save_owner_provider_profile(&b, &second).unwrap();

        assert_eq!(
            load_windows_owner_profile(&a)
                .unwrap()
                .unwrap()
                .worker_token,
            "worker-owner-a"
        );
        assert_eq!(
            load_windows_owner_profile(&b)
                .unwrap()
                .unwrap()
                .worker_token,
            "worker-owner-b"
        );
        assert!(save_owner_provider_profile(&a, &second).is_err());
        delete_owner_provider_profile(&a).unwrap();
        assert!(load_windows_owner_profile(&a).unwrap().is_none());
        assert_eq!(
            load_windows_owner_profile(&b)
                .unwrap()
                .unwrap()
                .worker_token,
            "worker-owner-b"
        );
        delete_owner_provider_profile(&b).unwrap();
        assert!(load_windows_owner_profile(&b).unwrap().is_none());
    }
}
