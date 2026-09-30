use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ReviewedOwnerContextSnapshot;

const STORE_SCHEMA: &str = "vpr-reviewed-owner-persona-1";
const STORE_PATH_ENV: &str = "VPR_OWNER_LAB_PERSONA_STORE_PATH";
#[cfg(windows)]
const WINDOWS_SERVICE: &str = "Virtual-Persona-Runtime";
#[cfg(windows)]
const WINDOWS_LEGACY_ACCOUNT: &str = "owner-lab-reviewed-persona-v1";
#[cfg(windows)]
const WINDOWS_STORE_PREFIX: &str = "owner-lab-reviewed-persona-v2";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedPersona {
    schema_version: String,
    snapshot: ReviewedOwnerContextSnapshot,
}

impl PersistedPersona {
    fn new(snapshot: ReviewedOwnerContextSnapshot) -> Self {
        Self {
            schema_version: STORE_SCHEMA.into(),
            snapshot,
        }
    }

    fn validate(self) -> Result<ReviewedOwnerContextSnapshot, String> {
        if self.schema_version != STORE_SCHEMA {
            return Err("reviewed Persona store schema is unsupported".into());
        }
        Ok(self.snapshot)
    }
}

/// Loads the reviewed Persona from durable storage when one exists.
///
/// # Errors
/// Returns a redacted error when the selected store cannot be read, decoded, or validated.
pub fn load_reviewed_persona() -> Result<Option<ReviewedOwnerContextSnapshot>, String> {
    if let Some(path) = explicit_store_path() {
        return load_file(&path);
    }

    #[cfg(windows)]
    {
        return platform::load();
    }

    #[cfg(not(windows))]
    {
        Ok(None)
    }
}

/// Persists the reviewed Persona to the canonical durable store.
///
/// # Errors
/// Returns a redacted error when serialization or the selected store write fails.
pub fn save_reviewed_persona(snapshot: &ReviewedOwnerContextSnapshot) -> Result<(), String> {
    if let Some(path) = explicit_store_path() {
        return save_file(&path, snapshot);
    }

    #[cfg(windows)]
    {
        return platform::save(snapshot);
    }

    #[cfg(not(windows))]
    {
        Ok(())
    }
}

fn explicit_store_path() -> Option<PathBuf> {
    env::var_os(STORE_PATH_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn load_file(path: &Path) -> Result<Option<ReviewedOwnerContextSnapshot>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("reviewed Persona file store read failed".into()),
    };
    let persisted: PersistedPersona = serde_json::from_str(&raw)
        .map_err(|_| "reviewed Persona file store contains invalid data")?;
    persisted.validate().map(Some)
}

fn save_file(path: &Path, snapshot: &ReviewedOwnerContextSnapshot) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|_| "reviewed Persona file store directory creation failed")?;
    }
    let raw = serde_json::to_vec_pretty(&PersistedPersona::new(snapshot.clone()))
        .map_err(|_| "reviewed Persona serialization failed")?;
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, raw).map_err(|_| "reviewed Persona file store write failed")?;
    fs::rename(&temporary, path)
        .map_err(|_| "reviewed Persona file store commit failed".to_string())
}

#[cfg(windows)]
mod platform {
    use super::{
        PersistedPersona, ReviewedOwnerContextSnapshot, WINDOWS_LEGACY_ACCOUNT, WINDOWS_SERVICE,
        WINDOWS_STORE_PREFIX,
    };
    use crate::windows_secure_store::ChunkedCredentialStore;

    fn store() -> ChunkedCredentialStore {
        ChunkedCredentialStore::new(
            WINDOWS_SERVICE,
            WINDOWS_STORE_PREFIX,
            Some(WINDOWS_LEGACY_ACCOUNT),
        )
    }

    pub(super) fn load() -> Result<Option<ReviewedOwnerContextSnapshot>, String> {
        let Some(raw) = store().load()? else {
            return Ok(None);
        };
        let persisted: PersistedPersona = serde_json::from_str(&raw)
            .map_err(|_| "Windows reviewed Persona store contains invalid data")?;
        persisted.validate().map(Some)
    }

    pub(super) fn save(snapshot: &ReviewedOwnerContextSnapshot) -> Result<(), String> {
        let raw = serde_json::to_string(&PersistedPersona::new(snapshot.clone()))
            .map_err(|_| "reviewed Persona serialization failed")?;
        store()
            .save(&raw)
            .map_err(|_| "Windows reviewed Persona store write failed".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sample() -> ReviewedOwnerContextSnapshot {
        ReviewedOwnerContextSnapshot {
            persona_id: "persisted-owner".into(),
            persona_version: 3,
            claims: vec![crate::ReviewedOwnerClaimSnapshot {
                claim_id: "preference-communication-style".into(),
                statement: "Кратко и по существу".into(),
                kind: "preference".into(),
                revision: 3,
            }],
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_chunked_store_round_trips_reviewed_persona_beyond_single_blob_limit() {
        use crate::windows_secure_store::ChunkedCredentialStore;

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let prefix = format!(
            "owner-lab-reviewed-persona-test-{}-{unique}",
            std::process::id()
        );
        let prefix: &'static str = Box::leak(prefix.into_boxed_str());
        let store = ChunkedCredentialStore::new(
            "Virtual-Persona-Runtime-Test",
            prefix,
            None,
        );
        let _ = store.delete();

        let mut large = sample();
        large.claims[0].statement = "Ж".repeat(4_000);
        let raw =
            serde_json::to_string(&PersistedPersona::new(large.clone())).expect("sample must serialize");
        assert!(raw.len() > 2_560);

        store.save(&raw).expect("chunked Credential Manager store must accept large Persona");
        let restored_raw = store
            .load()
            .expect("chunked Credential Manager store must read Persona")
            .expect("stored Persona must exist");
        let restored: PersistedPersona =
            serde_json::from_str(&restored_raw).expect("stored Persona must decode");
        assert_eq!(restored.validate().unwrap(), large);
        store.delete().unwrap();
    }

    #[test]
    fn explicit_file_store_round_trips_snapshot() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("vpr-persona-store-{unique}.json"));
        save_file(&path, &sample()).unwrap();
        assert_eq!(load_file(&path).unwrap(), Some(sample()));
        let _ = fs::remove_file(path);
    }
}
