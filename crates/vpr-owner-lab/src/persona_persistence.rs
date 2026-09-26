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
const WINDOWS_ACCOUNT: &str = "owner-lab-reviewed-persona-v1";

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
        PersistedPersona, ReviewedOwnerContextSnapshot, WINDOWS_ACCOUNT, WINDOWS_SERVICE,
    };
    use keyring::{Entry, Error as KeyringError};

    fn entry() -> Result<Entry, String> {
        Entry::new(WINDOWS_SERVICE, WINDOWS_ACCOUNT)
            .map_err(|_| "Windows reviewed Persona store initialization failed".into())
    }

    pub(super) fn load() -> Result<Option<ReviewedOwnerContextSnapshot>, String> {
        let raw = match entry()?.get_password() {
            Ok(raw) => raw,
            Err(KeyringError::NoEntry) => return Ok(None),
            Err(_) => return Err("Windows reviewed Persona store read failed".into()),
        };
        let persisted: PersistedPersona = serde_json::from_str(&raw)
            .map_err(|_| "Windows reviewed Persona store contains invalid data")?;
        persisted.validate().map(Some)
    }

    pub(super) fn save(snapshot: &ReviewedOwnerContextSnapshot) -> Result<(), String> {
        let raw = serde_json::to_string(&PersistedPersona::new(snapshot.clone()))
            .map_err(|_| "reviewed Persona serialization failed")?;
        entry()?
            .set_password(&raw)
            .map_err(|_| "Windows reviewed Persona store write failed".to_string())
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
