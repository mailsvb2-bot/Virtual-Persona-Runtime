use serde::{Deserialize, Serialize};

use crate::binding::{valid_git_sha, validate_provider_state};
use crate::exit_validation::validate_quality_latencies;
use crate::known_limitations::parse_known_limitations_review_status;
use crate::{
    CheckStatus, EvidenceOrigin, HumanDimensions, ParticipantRole, ProviderRole,
    ProviderStateManifest, QualityEvidence, RT0_PROVIDER_STATE_SCHEMA, RecordStatus, sha256_hex,
};

pub const RT0_SUPPORTING_PREFLIGHT_SCHEMA: &str = "rt0-supporting-evidence-preflight-0.1";

#[derive(Debug, Clone, Copy)]
pub struct Rt0SupportingPreflightArtifacts<'a> {
    pub ci: &'a [u8],
    pub e2e: &'a [u8],
    pub owner_conversation: &'a [u8],
    pub visitor_conversation: &'a [u8],
    pub acceptance: &'a [u8],
    pub quality: &'a [u8],
    pub cost: &'a [u8],
    pub privacy_permissions: &'a [u8],
    pub human_evaluation: &'a [u8],
    pub known_limitations: &'a [u8],
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Rt0SupportingArtifactDigests {
    pub ci: String,
    pub e2e: String,
    pub owner_conversation: String,
    pub visitor_conversation: String,
    pub acceptance: String,
    pub quality: String,
    pub cost: String,
    pub privacy_permissions: String,
    pub human_evaluation: String,
    pub known_limitations: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Rt0SupportingPreflightReport {
    pub schema_version: String,
    pub candidate_sha: String,
    pub provider_state_sha256: String,
    pub known_limitations_review_status: CheckStatus,
    pub preflight_complete: bool,
    pub artifact_digests: Rt0SupportingArtifactDigests,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Rt0SupportingPreflightError {
    InvalidCandidate,
    InvalidProviderState,
    InvalidJson,
    CandidateMismatch,
    ProviderStateMismatch,
    OriginNotReal,
    ParticipantRoleMismatch,
    InvalidQuality,
    IncompleteHumanReview,
    InvalidKnownLimitations,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AutomatedSupportingClaim {
    status: CheckStatus,
    candidate_sha: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConversationSupportingClaim {
    origin: EvidenceOrigin,
    role: ParticipantRole,
    russian: CheckStatus,
    voice: CheckStatus,
    video: CheckStatus,
    completed_turns: u32,
    interruption_exercised: CheckStatus,
    candidate_sha: String,
    provider_state_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptanceSupportingClaim {
    origin: EvidenceOrigin,
    owner_happy_path: CheckStatus,
    visitor_happy_path: CheckStatus,
    correction_path: CheckStatus,
    failure_recovery_path: CheckStatus,
    revoke_deny_path: CheckStatus,
    candidate_sha: String,
    provider_state_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QualitySupportingClaim {
    origin: EvidenceOrigin,
    text_first_meaningful_response: crate::LatencyDistributionMillis,
    first_meaningful_audio: crate::LatencyDistributionMillis,
    interruption_stop: crate::LatencyDistributionMillis,
    first_useful_video: crate::LatencyDistributionMillis,
    av_sync_absolute_offset: crate::LatencyDistributionMillis,
    recoverable_reconnect: crate::LatencyDistributionMillis,
    candidate_sha: String,
    provider_state_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CostSupportingClaim {
    origin: EvidenceOrigin,
    covered_provider_roles: Vec<ProviderRole>,
    measured_duration_millis: u64,
    estimated_cost_microunits: Option<u64>,
    provider_charge_microunits: Option<u64>,
    candidate_sha: String,
    provider_state_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivacySupportingClaim {
    origin: EvidenceOrigin,
    permission_suite: CheckStatus,
    accepted_private_context_leakage: u32,
    accepted_false_owner_attribution: u32,
    revocation: CheckStatus,
    egress_denial: CheckStatus,
    candidate_sha: String,
    provider_state_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HumanSupportingClaim {
    origin: EvidenceOrigin,
    rubric_version: String,
    reviewer_count: u32,
    dimensions: HumanDimensions,
    usable_for_continuation: CheckStatus,
    candidate_sha: String,
    provider_state_sha256: String,
}

/// Performs a non-promoting structural and binding preflight over the ten RT0 supporting artifacts.
///
/// The report intentionally contains no release-ready decision. Substantive pass/fail outcomes are
/// preserved for the canonical RT0 exit gate.
///
/// # Errors
/// Returns a stable preflight error when the candidate/provider binding is stale, JSON is malformed,
/// a real-evidence slot is not real, owner/visitor roles drift, quality distributions are invalid,
/// mandatory human-review records are incomplete, or known-limitations evidence is invalid.
pub fn preflight_rt0_supporting_artifacts(
    artifacts: Rt0SupportingPreflightArtifacts<'_>,
    provider_state_bytes: &[u8],
    exact_candidate_sha: &str,
) -> Result<Rt0SupportingPreflightReport, Rt0SupportingPreflightError> {
    if !valid_git_sha(exact_candidate_sha) {
        return Err(Rt0SupportingPreflightError::InvalidCandidate);
    }
    let provider_state: ProviderStateManifest = serde_json::from_slice(provider_state_bytes)
        .map_err(|_| Rt0SupportingPreflightError::InvalidProviderState)?;
    if provider_state.schema_version != RT0_PROVIDER_STATE_SCHEMA
        || validate_provider_state(&provider_state.providers).is_err()
    {
        return Err(Rt0SupportingPreflightError::InvalidProviderState);
    }
    let provider_state_sha256 = sha256_hex(provider_state_bytes);

    validate_automated(artifacts.ci, exact_candidate_sha)?;
    validate_automated(artifacts.e2e, exact_candidate_sha)?;
    validate_conversation(
        artifacts.owner_conversation,
        exact_candidate_sha,
        &provider_state_sha256,
        ParticipantRole::Owner,
    )?;
    validate_conversation(
        artifacts.visitor_conversation,
        exact_candidate_sha,
        &provider_state_sha256,
        ParticipantRole::Visitor,
    )?;
    validate_acceptance(
        artifacts.acceptance,
        exact_candidate_sha,
        &provider_state_sha256,
    )?;
    validate_quality(
        artifacts.quality,
        exact_candidate_sha,
        &provider_state_sha256,
    )?;
    validate_cost(artifacts.cost, exact_candidate_sha, &provider_state_sha256)?;
    validate_privacy(
        artifacts.privacy_permissions,
        exact_candidate_sha,
        &provider_state_sha256,
    )?;
    validate_human(
        artifacts.human_evaluation,
        exact_candidate_sha,
        &provider_state_sha256,
    )?;
    let known_limitations_review_status =
        parse_known_limitations_review_status(artifacts.known_limitations)
            .map_err(|()| Rt0SupportingPreflightError::InvalidKnownLimitations)?;

    Ok(Rt0SupportingPreflightReport {
        schema_version: RT0_SUPPORTING_PREFLIGHT_SCHEMA.into(),
        candidate_sha: exact_candidate_sha.into(),
        provider_state_sha256,
        known_limitations_review_status,
        preflight_complete: true,
        artifact_digests: Rt0SupportingArtifactDigests {
            ci: sha256_hex(artifacts.ci),
            e2e: sha256_hex(artifacts.e2e),
            owner_conversation: sha256_hex(artifacts.owner_conversation),
            visitor_conversation: sha256_hex(artifacts.visitor_conversation),
            acceptance: sha256_hex(artifacts.acceptance),
            quality: sha256_hex(artifacts.quality),
            cost: sha256_hex(artifacts.cost),
            privacy_permissions: sha256_hex(artifacts.privacy_permissions),
            human_evaluation: sha256_hex(artifacts.human_evaluation),
            known_limitations: sha256_hex(artifacts.known_limitations),
        },
    })
}

fn validate_automated(
    bytes: &[u8],
    exact_candidate_sha: &str,
) -> Result<(), Rt0SupportingPreflightError> {
    let claim: AutomatedSupportingClaim =
        serde_json::from_slice(bytes).map_err(|_| Rt0SupportingPreflightError::InvalidJson)?;
    let _ = claim.status;
    validate_candidate(&claim.candidate_sha, exact_candidate_sha)
}

fn validate_conversation(
    bytes: &[u8],
    candidate_sha: &str,
    provider_state_sha256: &str,
    role: ParticipantRole,
) -> Result<(), Rt0SupportingPreflightError> {
    let claim: ConversationSupportingClaim =
        serde_json::from_slice(bytes).map_err(|_| Rt0SupportingPreflightError::InvalidJson)?;
    validate_real_binding(
        claim.origin,
        &claim.candidate_sha,
        &claim.provider_state_sha256,
        candidate_sha,
        provider_state_sha256,
    )?;
    if claim.role != role {
        return Err(Rt0SupportingPreflightError::ParticipantRoleMismatch);
    }
    let _ = (
        claim.russian,
        claim.voice,
        claim.video,
        claim.completed_turns,
        claim.interruption_exercised,
    );
    Ok(())
}

fn validate_acceptance(
    bytes: &[u8],
    candidate_sha: &str,
    provider_state_sha256: &str,
) -> Result<(), Rt0SupportingPreflightError> {
    let claim: AcceptanceSupportingClaim =
        serde_json::from_slice(bytes).map_err(|_| Rt0SupportingPreflightError::InvalidJson)?;
    validate_real_binding(
        claim.origin,
        &claim.candidate_sha,
        &claim.provider_state_sha256,
        candidate_sha,
        provider_state_sha256,
    )?;
    let _ = (
        claim.owner_happy_path,
        claim.visitor_happy_path,
        claim.correction_path,
        claim.failure_recovery_path,
        claim.revoke_deny_path,
    );
    Ok(())
}

fn validate_quality(
    bytes: &[u8],
    candidate_sha: &str,
    provider_state_sha256: &str,
) -> Result<(), Rt0SupportingPreflightError> {
    let claim: QualitySupportingClaim =
        serde_json::from_slice(bytes).map_err(|_| Rt0SupportingPreflightError::InvalidJson)?;
    validate_real_binding(
        claim.origin,
        &claim.candidate_sha,
        &claim.provider_state_sha256,
        candidate_sha,
        provider_state_sha256,
    )?;
    let quality = QualityEvidence {
        origin: claim.origin,
        text_first_meaningful_response: claim.text_first_meaningful_response,
        first_meaningful_audio: claim.first_meaningful_audio,
        interruption_stop: claim.interruption_stop,
        first_useful_video: claim.first_useful_video,
        av_sync_absolute_offset: claim.av_sync_absolute_offset,
        recoverable_reconnect: claim.recoverable_reconnect,
        artifact_sha256: "0".repeat(64),
    };
    validate_quality_latencies(&quality).map_err(|_| Rt0SupportingPreflightError::InvalidQuality)
}

fn validate_cost(
    bytes: &[u8],
    candidate_sha: &str,
    provider_state_sha256: &str,
) -> Result<(), Rt0SupportingPreflightError> {
    let claim: CostSupportingClaim =
        serde_json::from_slice(bytes).map_err(|_| Rt0SupportingPreflightError::InvalidJson)?;
    validate_real_binding(
        claim.origin,
        &claim.candidate_sha,
        &claim.provider_state_sha256,
        candidate_sha,
        provider_state_sha256,
    )?;
    let _ = (
        claim.covered_provider_roles,
        claim.measured_duration_millis,
        claim.estimated_cost_microunits,
        claim.provider_charge_microunits,
    );
    Ok(())
}

fn validate_privacy(
    bytes: &[u8],
    candidate_sha: &str,
    provider_state_sha256: &str,
) -> Result<(), Rt0SupportingPreflightError> {
    let claim: PrivacySupportingClaim =
        serde_json::from_slice(bytes).map_err(|_| Rt0SupportingPreflightError::InvalidJson)?;
    validate_real_binding(
        claim.origin,
        &claim.candidate_sha,
        &claim.provider_state_sha256,
        candidate_sha,
        provider_state_sha256,
    )?;
    let _ = (
        claim.permission_suite,
        claim.accepted_private_context_leakage,
        claim.accepted_false_owner_attribution,
        claim.revocation,
        claim.egress_denial,
    );
    Ok(())
}

fn validate_human(
    bytes: &[u8],
    candidate_sha: &str,
    provider_state_sha256: &str,
) -> Result<(), Rt0SupportingPreflightError> {
    let claim: HumanSupportingClaim =
        serde_json::from_slice(bytes).map_err(|_| Rt0SupportingPreflightError::InvalidJson)?;
    validate_real_binding(
        claim.origin,
        &claim.candidate_sha,
        &claim.provider_state_sha256,
        candidate_sha,
        provider_state_sha256,
    )?;
    if claim.rubric_version.trim().is_empty()
        || claim.reviewer_count == 0
        || !human_dimensions_complete(&claim.dimensions)
    {
        return Err(Rt0SupportingPreflightError::IncompleteHumanReview);
    }
    let _ = claim.usable_for_continuation;
    Ok(())
}

fn validate_candidate(
    candidate_sha: &str,
    exact_candidate_sha: &str,
) -> Result<(), Rt0SupportingPreflightError> {
    if !valid_git_sha(candidate_sha) {
        return Err(Rt0SupportingPreflightError::InvalidCandidate);
    }
    if candidate_sha != exact_candidate_sha {
        return Err(Rt0SupportingPreflightError::CandidateMismatch);
    }
    Ok(())
}

fn validate_real_binding(
    origin: EvidenceOrigin,
    candidate_sha: &str,
    provider_state_sha256: &str,
    exact_candidate_sha: &str,
    exact_provider_state_sha256: &str,
) -> Result<(), Rt0SupportingPreflightError> {
    if origin != EvidenceOrigin::Real {
        return Err(Rt0SupportingPreflightError::OriginNotReal);
    }
    validate_candidate(candidate_sha, exact_candidate_sha)?;
    if provider_state_sha256 != exact_provider_state_sha256 {
        return Err(Rt0SupportingPreflightError::ProviderStateMismatch);
    }
    Ok(())
}

fn human_dimensions_complete(dimensions: &HumanDimensions) -> bool {
    [
        dimensions.voice_similarity,
        dimensions.voice_naturalness,
        dimensions.appearance_plausibility,
        dimensions.persona_similarity,
        dimensions.conversation_naturalness,
    ]
    .into_iter()
    .all(|status| status == RecordStatus::Recorded)
}
