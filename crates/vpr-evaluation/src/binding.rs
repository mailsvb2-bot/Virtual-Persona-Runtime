use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const RT0_EVIDENCE_BINDING_SCHEMA: &str = "rt0-evidence-binding-0.1";
pub const RT0_PROVIDER_STATE_SCHEMA: &str = "rt0-provider-state-0.1";

use crate::{
    GoldenObservation, GoldenReport, GoldenSuite, GoldenSuiteError, evaluate_golden_suite,
};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRole {
    Stt,
    Llm,
    Avatar,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderStateBinding {
    pub role: ProviderRole,
    pub provider: String,
    pub model_or_representation: String,
    pub configuration_fingerprint_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderStateManifest {
    pub schema_version: String,
    pub providers: Vec<ProviderStateBinding>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EvidenceBinding {
    pub schema_version: String,
    pub candidate_sha: String,
    pub release_spec_sha256: String,
    pub suite_sha256: String,
    pub provider_state_sha256: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GoldenEvidenceBundle {
    pub binding: EvidenceBinding,
    pub observations: Vec<GoldenObservation>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BoundGoldenReport {
    pub evidence_input_sha256: String,
    pub binding: EvidenceBinding,
    pub provider_state: ProviderStateManifest,
    pub golden: GoldenReport,
}

#[derive(Debug, Clone, Copy)]
pub struct EvidenceVerificationContext<'a> {
    pub suite_bytes: &'a [u8],
    pub release_spec_bytes: &'a [u8],
    pub provider_state: &'a ProviderStateManifest,
    pub provider_state_bytes: &'a [u8],
    pub evidence_bytes: &'a [u8],
    pub exact_candidate_sha: &'a str,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EvidenceBindingError {
    UnsupportedBindingSchema,
    UnsupportedProviderStateSchema,
    InvalidCandidateSha,
    InvalidDigest,
    CandidateShaMismatch,
    SuiteDigestMismatch,
    ReleaseSpecDigestMismatch,
    ProviderStateDigestMismatch,
    MissingProviderRole,
    DuplicateProviderRole,
    BlankProviderDescriptor,
    InvalidProviderFingerprint,
    GoldenSuiteInvalid,
}

impl From<GoldenSuiteError> for EvidenceBindingError {
    fn from(_: GoldenSuiteError) -> Self {
        Self::GoldenSuiteInvalid
    }
}

/// Computes a lowercase SHA-256 digest for one evidence artifact.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("{digest:x}")
}

/// Verifies exact-candidate, artifact and provider-state binding, then evaluates the Golden Set.
///
/// # Errors
/// Returns a stable `EvidenceBindingError` when any binding field is malformed or stale, when the
/// provider-state manifest is incomplete, or when the underlying Golden Set is structurally invalid.
pub fn evaluate_bound_golden_suite(
    suite: &GoldenSuite,
    bundle: &GoldenEvidenceBundle,
    context: EvidenceVerificationContext<'_>,
) -> Result<BoundGoldenReport, EvidenceBindingError> {
    validate_binding(
        &bundle.binding,
        context.exact_candidate_sha,
        context.suite_bytes,
        context.release_spec_bytes,
        context.provider_state,
        context.provider_state_bytes,
    )?;
    let golden = evaluate_golden_suite(suite, &bundle.observations)?;
    Ok(BoundGoldenReport {
        evidence_input_sha256: sha256_hex(context.evidence_bytes),
        binding: bundle.binding.clone(),
        provider_state: context.provider_state.clone(),
        golden,
    })
}

fn validate_binding(
    binding: &EvidenceBinding,
    exact_candidate_sha: &str,
    suite_bytes: &[u8],
    release_spec_bytes: &[u8],
    provider_state: &ProviderStateManifest,
    provider_state_bytes: &[u8],
) -> Result<(), EvidenceBindingError> {
    if binding.schema_version != RT0_EVIDENCE_BINDING_SCHEMA {
        return Err(EvidenceBindingError::UnsupportedBindingSchema);
    }
    if provider_state.schema_version != RT0_PROVIDER_STATE_SCHEMA {
        return Err(EvidenceBindingError::UnsupportedProviderStateSchema);
    }
    if !valid_git_sha(exact_candidate_sha) || !valid_git_sha(&binding.candidate_sha) {
        return Err(EvidenceBindingError::InvalidCandidateSha);
    }
    for digest in [
        &binding.release_spec_sha256,
        &binding.suite_sha256,
        &binding.provider_state_sha256,
    ] {
        if !valid_sha256(digest) {
            return Err(EvidenceBindingError::InvalidDigest);
        }
    }
    if binding.candidate_sha != exact_candidate_sha {
        return Err(EvidenceBindingError::CandidateShaMismatch);
    }
    if binding.suite_sha256 != sha256_hex(suite_bytes) {
        return Err(EvidenceBindingError::SuiteDigestMismatch);
    }
    if binding.release_spec_sha256 != sha256_hex(release_spec_bytes) {
        return Err(EvidenceBindingError::ReleaseSpecDigestMismatch);
    }
    if binding.provider_state_sha256 != sha256_hex(provider_state_bytes) {
        return Err(EvidenceBindingError::ProviderStateDigestMismatch);
    }
    validate_provider_state(&provider_state.providers)
}

fn validate_provider_state(providers: &[ProviderStateBinding]) -> Result<(), EvidenceBindingError> {
    let mut roles = HashSet::new();
    for provider in providers {
        if !roles.insert(provider.role) {
            return Err(EvidenceBindingError::DuplicateProviderRole);
        }
        if provider.provider.trim().is_empty() || provider.model_or_representation.trim().is_empty()
        {
            return Err(EvidenceBindingError::BlankProviderDescriptor);
        }
        if !valid_sha256(&provider.configuration_fingerprint_sha256) {
            return Err(EvidenceBindingError::InvalidProviderFingerprint);
        }
    }
    for required in [ProviderRole::Stt, ProviderRole::Llm, ProviderRole::Avatar] {
        if !roles.contains(&required) {
            return Err(EvidenceBindingError::MissingProviderRole);
        }
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(is_lower_hex)
}

fn valid_git_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(is_lower_hex)
}

const fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')
}
