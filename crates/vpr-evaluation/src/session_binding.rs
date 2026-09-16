use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::binding::{valid_git_sha, validate_provider_state};
use crate::{
    LabSessionEvidenceAggregate, LabSessionEvidenceSnapshot, ProviderStateManifest,
    RT0_PROVIDER_STATE_SCHEMA, aggregate_owner_lab_session_evidence, sha256_hex,
};

pub const RT0_OWNER_LAB_SESSION_BINDING_SCHEMA: &str =
    "rt0-owner-lab-session-aggregate-binding-0.4";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BoundLabSessionEvidenceAggregate {
    pub schema_version: String,
    pub candidate_sha: String,
    pub provider_state_sha256: String,
    pub snapshot_sha256: Vec<String>,
    pub aggregate: LabSessionEvidenceAggregate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabSessionBindingError {
    EmptyInput,
    InvalidCandidateSha,
    InvalidProviderState,
    InvalidSnapshot,
    DuplicateArtifact,
    AggregateInvalid,
}

/// Parses, validates and binds sanitized Owner Lab session snapshots to one exact candidate and
/// one exact provider-state artifact.
///
/// The binding is intentionally evidence-only. It preserves runtime-backed canonical playback
/// proof when every completed voice attempt was reconciled to `Played`, but browser observations
/// alone cannot create that proof. Schema `0.4` also preserves request-scoped A/V sync evidence and the server-derived participant
/// role carried by each raw session snapshot. It does not infer participant identity from filenames.
///
/// # Errors
/// Returns a fail-closed error for malformed candidate/provider state/snapshots, duplicate raw
/// artifacts, or any aggregate contract violation.
pub fn bind_owner_lab_session_evidence(
    snapshot_artifacts: &[&[u8]],
    provider_state_bytes: &[u8],
    exact_candidate_sha: &str,
) -> Result<BoundLabSessionEvidenceAggregate, LabSessionBindingError> {
    if snapshot_artifacts.is_empty() {
        return Err(LabSessionBindingError::EmptyInput);
    }
    if !valid_git_sha(exact_candidate_sha) {
        return Err(LabSessionBindingError::InvalidCandidateSha);
    }

    let provider_state: ProviderStateManifest = serde_json::from_slice(provider_state_bytes)
        .map_err(|_| LabSessionBindingError::InvalidProviderState)?;
    if provider_state.schema_version != RT0_PROVIDER_STATE_SCHEMA
        || validate_provider_state(&provider_state.providers).is_err()
    {
        return Err(LabSessionBindingError::InvalidProviderState);
    }

    let mut artifact_digests = HashSet::new();
    let mut ordered_digests = Vec::with_capacity(snapshot_artifacts.len());
    let mut snapshots = Vec::with_capacity(snapshot_artifacts.len());
    for bytes in snapshot_artifacts {
        let digest = sha256_hex(bytes);
        if !artifact_digests.insert(digest.clone()) {
            return Err(LabSessionBindingError::DuplicateArtifact);
        }
        let snapshot: LabSessionEvidenceSnapshot =
            serde_json::from_slice(bytes).map_err(|_| LabSessionBindingError::InvalidSnapshot)?;
        ordered_digests.push(digest);
        snapshots.push(snapshot);
    }

    let aggregate = aggregate_owner_lab_session_evidence(&snapshots)
        .map_err(|_| LabSessionBindingError::AggregateInvalid)?;

    Ok(BoundLabSessionEvidenceAggregate {
        schema_version: RT0_OWNER_LAB_SESSION_BINDING_SCHEMA.into(),
        candidate_sha: exact_candidate_sha.into(),
        provider_state_sha256: sha256_hex(provider_state_bytes),
        snapshot_sha256: ordered_digests,
        aggregate,
    })
}
