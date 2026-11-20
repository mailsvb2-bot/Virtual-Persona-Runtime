use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ReviewedOwnerContextSnapshot;
use crate::owner_capture::Rt0OwnerCapture;
use crate::owner_context::{
    DurableOwnerClaimRevisionSnapshot, DurableReviewedOwnerClaimSnapshot,
    DurableReviewedOwnerContextSnapshot, ReviewedOwnerContext,
    durable_snapshot_from_reviewed_profile,
};
use crate::persona_file_io;
use crate::persona_transaction;
use crate::rt1_workspace::OwnerWorkspaceRecord;
use crate::state::OwnerLabEngine;

const STORE_SCHEMA_V1: &str = "vpr-reviewed-owner-persona-1";
const STORE_SCHEMA_V2: &str = "vpr-reviewed-owner-persona-2";
const STORE_PATH_ENV: &str = "VPR_OWNER_LAB_PERSONA_STORE_PATH";
#[cfg(windows)]
const WINDOWS_SERVICE: &str = "Virtual-Persona-Runtime";
#[cfg(windows)]
const WINDOWS_LEGACY_ACCOUNT: &str = "owner-lab-reviewed-persona-v1";
#[cfg(windows)]
const WINDOWS_STORE_PREFIX: &str = "owner-lab-reviewed-persona-v2";

#[derive(Debug, Deserialize)]
struct SchemaProbe {
    schema_version: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedPersonaV1 {
    schema_version: String,
    snapshot: ReviewedOwnerContextSnapshot,
}

/// An additional owner has its own reviewed Persona and publication state,
/// retained in the SAME atomic canonical envelope as the legacy owner.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdditionalOwnerPersona {
    pub(crate) snapshot: DurableReviewedOwnerContextSnapshot,
    pub(crate) workspace: OwnerWorkspaceRecord,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PersistedPersonaV2 {
    schema_version: String,
    pub(crate) snapshot: DurableReviewedOwnerContextSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) workspace: Option<OwnerWorkspaceRecord>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) additional_owner_personas: BTreeMap<String, AdditionalOwnerPersona>,
}

impl PersistedPersonaV2 {
    pub(crate) fn new(snapshot: DurableReviewedOwnerContextSnapshot) -> Self {
        Self {
            schema_version: STORE_SCHEMA_V2.into(),
            snapshot,
            workspace: None,
            additional_owner_personas: BTreeMap::new(),
        }
    }

    fn reconciled(
        snapshot: DurableReviewedOwnerContextSnapshot,
        previous: Option<&str>,
    ) -> Result<Self, String> {
        let mut next = Self::new(snapshot);
        if let Some(previous) = previous {
            let probe: SchemaProbe =
                serde_json::from_str(previous).map_err(|_| "existing Persona record is invalid")?;
            if probe.schema_version == STORE_SCHEMA_V2 {
                let old: Self = serde_json::from_str(previous)
                    .map_err(|_| "existing Persona v2 record is invalid")?;
                old.clone().validate()?;
                if old.snapshot.persona_id != next.snapshot.persona_id
                    || old.snapshot.persona_version > next.snapshot.persona_version
                {
                    return Err("canonical Persona identity or version mismatch".into());
                }
                // A competing writer must never replace reviewed content at
                // the same PersonaVersion. Identical retries remain safe.
                if old.snapshot.persona_version == next.snapshot.persona_version
                    && old.snapshot != next.snapshot
                {
                    return Err("canonical Persona same-version write conflict".into());
                }
                next.workspace = old.workspace;
                next.additional_owner_personas = old.additional_owner_personas;
                if old.snapshot.persona_version < next.snapshot.persona_version {
                    if let Some(workspace) = next.workspace.as_mut() {
                        workspace.on_persona_correction(next.snapshot.persona_version)?;
                    }
                }
            } else if probe.schema_version != STORE_SCHEMA_V1 {
                return Err("existing Persona schema is unsupported".into());
            }
        }
        next.clone().validate()?;
        Ok(next)
    }

    pub(crate) fn validate(self) -> Result<DurableReviewedOwnerContextSnapshot, String> {
        if self.schema_version != STORE_SCHEMA_V2 {
            return Err("reviewed Persona store schema is unsupported".into());
        }
        let mut known_owner_ids = HashSet::new();
        let mut known_persona_ids = HashSet::new();
        known_persona_ids.insert(self.snapshot.persona_id.as_str());
        if let Some(workspace) = &self.workspace {
            workspace.validate(&self.snapshot.persona_id, self.snapshot.persona_version)?;
            known_owner_ids.insert(workspace.owner_id.as_str());
        }
        for (owner_id, record) in &self.additional_owner_personas {
            // Never trust a map key as authority: it must match the owner
            // identity sealed into that exact Persona publication record.
            if owner_id != &record.workspace.owner_id
                || !known_owner_ids.insert(owner_id.as_str())
                || !known_persona_ids.insert(record.snapshot.persona_id.as_str())
            {
                return Err("duplicate or mismatched canonical owner/Persona identity".into());
            }
            ReviewedOwnerContext::from_durable_snapshot(&record.snapshot)
                .map_err(|_| "additional reviewed Persona is invalid")?;
            record.workspace.validate(
                &record.snapshot.persona_id,
                record.snapshot.persona_version,
            )?;
        }
        Ok(self.snapshot)
    }
}

struct DecodedPersona {
    snapshot: DurableReviewedOwnerContextSnapshot,
    migrated_legacy: bool,
}

/// Restores a durable reviewed Persona directly into the canonical engine.
/// The persistence DTO never crosses the public API boundary.
///
/// # Errors
/// Returns a redacted persistence or validation error and leaves the engine without restored
/// owner context.
pub fn restore_reviewed_persona(engine: &mut OwnerLabEngine) -> Result<bool, String> {
    let Some(snapshot) = load_reviewed_persona()? else {
        return Ok(false);
    };
    engine
        .restore_reviewed_owner_context(&snapshot)
        .map_err(|_| "persisted reviewed Persona is invalid".to_owned())?;
    Ok(true)
}

/// Persists a completed reviewed capture using the exact retained domain history.
/// # Errors
/// Returns a redacted validation or durable-store error.
pub fn persist_reviewed_capture(capture: &Rt0OwnerCapture) -> Result<(), String> {
    let snapshot = durable_snapshot_from_reviewed_profile(capture.profile())
        .map_err(|_| "reviewed Persona capture is invalid".to_owned())?;
    save_reviewed_persona(&snapshot)
}
/// Loads the reviewed Persona from durable storage when one exists.
///
/// # Errors
/// Returns a redacted error when the selected store cannot be read, decoded, or validated.
pub(crate) fn load_reviewed_persona() -> Result<Option<DurableReviewedOwnerContextSnapshot>, String>
{
    if let Some(path) = explicit_store_path() {
        return load_file(&path);
    }

    #[cfg(windows)]
    {
        let _transaction = persona_transaction::begin()?;
        platform::load()
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
pub(crate) fn save_reviewed_persona(
    snapshot: &DurableReviewedOwnerContextSnapshot,
) -> Result<(), String> {
    if let Some(path) = explicit_store_path() {
        return save_file(&path, snapshot);
    }

    #[cfg(windows)]
    {
        let _transaction = persona_transaction::begin()?;
        platform::save(snapshot)
    }

    #[cfg(not(windows))]
    {
        Err("reviewed Persona durable store path must be configured".into())
    }
}

fn decode_persisted(raw: &str) -> Result<DecodedPersona, String> {
    let probe: SchemaProbe =
        serde_json::from_str(raw).map_err(|_| "reviewed Persona store contains invalid data")?;
    match probe.schema_version.as_str() {
        STORE_SCHEMA_V2 => {
            let persisted: PersistedPersonaV2 = serde_json::from_str(raw)
                .map_err(|_| "reviewed Persona store contains invalid v2 data")?;
            Ok(DecodedPersona {
                snapshot: persisted.validate()?,
                migrated_legacy: false,
            })
        }
        STORE_SCHEMA_V1 => {
            let persisted: PersistedPersonaV1 = serde_json::from_str(raw)
                .map_err(|_| "reviewed Persona store contains invalid v1 data")?;
            if persisted.schema_version != STORE_SCHEMA_V1 {
                return Err("reviewed Persona store schema is unsupported".into());
            }
            Ok(DecodedPersona {
                snapshot: migrate_v1_snapshot(persisted.snapshot)?,
                migrated_legacy: true,
            })
        }
        _ => Err("reviewed Persona store schema is unsupported".into()),
    }
}

fn migrate_v1_snapshot(
    snapshot: ReviewedOwnerContextSnapshot,
) -> Result<DurableReviewedOwnerContextSnapshot, String> {
    if snapshot.persona_version < 2 || snapshot.claims.is_empty() {
        return Err("legacy reviewed Persona snapshot is invalid".into());
    }
    let mut claims = Vec::with_capacity(snapshot.claims.len());
    for claim in snapshot.claims {
        if claim.revision < 2 || claim.revision > 10_000 {
            return Err("legacy reviewed Persona claim revision is invalid".into());
        }
        claims.push(DurableReviewedOwnerClaimSnapshot {
            claim_id: claim.claim_id,
            history_complete: false,
            revisions: vec![DurableOwnerClaimRevisionSnapshot {
                revision: claim.revision,
                statement: claim.statement,
                kind: claim.kind,
                source: "owner".into(),
                verification: "owner_verified".into(),
                derivation: "direct".into(),
            }],
        });
    }
    let durable = DurableReviewedOwnerContextSnapshot {
        persona_id: snapshot.persona_id,
        persona_version: snapshot.persona_version,
        claims,
    };
    let validated = ReviewedOwnerContext::from_durable_snapshot(&durable)
        .map_err(|_| "legacy reviewed Persona snapshot is invalid".to_owned())?;
    Ok(validated.durable_snapshot())
}
fn explicit_store_path() -> Option<PathBuf> {
    env::var_os(STORE_PATH_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn load_file(path: &Path) -> Result<Option<DurableReviewedOwnerContextSnapshot>, String> {
    // A legacy migration is a read-modify-commit and must not race a
    // workspace publication from another process.
    let _transaction = persona_transaction::begin_for_path(path)?;
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("reviewed Persona file store read failed".into()),
    };
    let decoded = decode_persisted(&raw)?;
    if decoded.migrated_legacy {
        save_file_unlocked(path, &decoded.snapshot)?;
    }
    Ok(Some(decoded.snapshot))
}

fn save_file(path: &Path, snapshot: &DurableReviewedOwnerContextSnapshot) -> Result<(), String> {
    let _transaction = persona_transaction::begin_for_path(path)?;
    save_file_unlocked(path, snapshot)
}

fn save_file_unlocked(
    path: &Path,
    snapshot: &DurableReviewedOwnerContextSnapshot,
) -> Result<(), String> {
    let previous = persona_file_io::read_raw(path)?;
    let record = PersistedPersonaV2::reconciled(snapshot.clone(), previous.as_deref())?;
    let raw =
        serde_json::to_vec_pretty(&record).map_err(|_| "reviewed Persona serialization failed")?;
    persona_file_io::commit_raw(path, &raw)
}

pub(crate) fn load_current_record() -> Result<Option<PersistedPersonaV2>, String> {
    let raw = if let Some(path) = explicit_store_path() {
        persona_file_io::read_raw(&path)?
    } else {
        #[cfg(windows)]
        {
            platform::load_raw()?
        }
        #[cfg(not(windows))]
        {
            return Err("reviewed Persona durable store path must be configured".into());
        }
    };
    let Some(raw) = raw else {
        return Ok(None);
    };
    let record: PersistedPersonaV2 =
        serde_json::from_str(&raw).map_err(|_| "canonical Persona v2 record required")?;
    record.clone().validate()?;
    Ok(Some(record))
}

pub(crate) fn commit_current_record(record: &PersistedPersonaV2) -> Result<(), String> {
    record.clone().validate()?;
    let raw =
        serde_json::to_vec_pretty(record).map_err(|_| "reviewed Persona serialization failed")?;
    if let Some(path) = explicit_store_path() {
        persona_file_io::commit_raw(&path, &raw)
    } else {
        #[cfg(windows)]
        {
            let utf8 = std::str::from_utf8(&raw).map_err(|_| "reviewed Persona encoding failed")?;
            platform::save_raw(utf8)
        }
        #[cfg(not(windows))]
        {
            Err("reviewed Persona durable store path must be configured".into())
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::{
        DurableReviewedOwnerContextSnapshot, PersistedPersonaV2, WINDOWS_LEGACY_ACCOUNT,
        WINDOWS_SERVICE, WINDOWS_STORE_PREFIX, decode_persisted,
    };
    use crate::windows_secure_store::ChunkedCredentialStore;

    fn store() -> ChunkedCredentialStore {
        ChunkedCredentialStore::new(
            WINDOWS_SERVICE,
            WINDOWS_STORE_PREFIX,
            Some(WINDOWS_LEGACY_ACCOUNT),
        )
    }

    pub(super) fn load_raw() -> Result<Option<String>, String> {
        store().load()
    }

    pub(super) fn save_raw(raw: &str) -> Result<(), String> {
        store()
            .save(raw)
            .map_err(|_| "Windows reviewed Persona store write failed".to_owned())
    }

    pub(super) fn load() -> Result<Option<DurableReviewedOwnerContextSnapshot>, String> {
        let Some(raw) = store().load()? else {
            return Ok(None);
        };
        let decoded = decode_persisted(&raw)
            .map_err(|_| "Windows reviewed Persona store contains invalid data".to_owned())?;
        if decoded.migrated_legacy {
            save(&decoded.snapshot)?;
        }
        Ok(Some(decoded.snapshot))
    }

    pub(super) fn save(snapshot: &DurableReviewedOwnerContextSnapshot) -> Result<(), String> {
        let previous = store().load()?;
        let record = PersistedPersonaV2::reconciled(snapshot.clone(), previous.as_deref())?;
        let raw =
            serde_json::to_string(&record).map_err(|_| "reviewed Persona serialization failed")?;
        save_raw(&raw)
    }
}

#[cfg(test)]
#[path = "persona_persistence_tests.rs"]
mod tests;
