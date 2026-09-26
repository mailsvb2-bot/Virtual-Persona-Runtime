use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::binding::{
    EvidenceVerificationContext, evaluate_bound_golden_suite, valid_git_sha, valid_sha256,
    validate_provider_state,
};
use crate::exit_context::Rt0ExitVerificationContext;
use crate::exit_validation::validate_runtime_evidence;
use crate::live_provider::{LiveProviderProbeValidationError, validate_live_provider_probe};
use crate::{
    BoundGoldenReport, GoldenSuite, RT0_EVIDENCE_BINDING_SCHEMA, RT0_GOLDEN_SCHEMA,
    RT0_PROVIDER_STATE_SCHEMA, sha256_hex,
};

pub const RT0_EXIT_EVIDENCE_SCHEMA: &str = "rt0-exit-evidence-0.4";
pub const RT0_EXIT_REPORT_SCHEMA: &str = "rt0-exit-report-0.4";
const RT0_REQUIRED_GOLDEN_SUITE_BYTES: &[u8] =
    include_bytes!("../../../docs/evaluation/rt0_golden_minimum.json");

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceOrigin {
    Real,
    Mock,
    Synthetic,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParticipantRole {
    Owner,
    Visitor,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Passed,
    Failed,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecordStatus {
    Recorded,
    Missing,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactCheckEvidence {
    pub status: CheckStatus,
    pub artifact_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AutomatedEvidence {
    pub ci: ArtifactCheckEvidence,
    pub e2e: ArtifactCheckEvidence,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationEvidence {
    pub origin: EvidenceOrigin,
    pub role: ParticipantRole,
    pub russian: CheckStatus,
    pub voice: CheckStatus,
    pub video: CheckStatus,
    pub completed_turns: u32,
    pub interruption_exercised: CheckStatus,
    pub artifact_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationPairEvidence {
    pub owner: ConversationEvidence,
    pub visitor: ConversationEvidence,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LatencyDistributionMillis {
    pub samples: u32,
    pub p50: u64,
    pub p95: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QualityEvidence {
    pub origin: EvidenceOrigin,
    pub text_first_meaningful_response: LatencyDistributionMillis,
    pub first_meaningful_audio: LatencyDistributionMillis,
    pub interruption_stop: LatencyDistributionMillis,
    pub first_useful_video: LatencyDistributionMillis,
    pub av_sync_absolute_offset: LatencyDistributionMillis,
    pub recoverable_reconnect: LatencyDistributionMillis,
    pub artifact_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CostEvidence {
    pub origin: EvidenceOrigin,
    pub measured_duration_millis: u64,
    pub estimated_cost_microunits: Option<u64>,
    pub provider_charge_microunits: Option<u64>,
    pub artifact_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PrivacyPermissionEvidence {
    pub origin: EvidenceOrigin,
    pub permission_suite: CheckStatus,
    pub accepted_private_context_leakage: u32,
    pub accepted_false_owner_attribution: u32,
    pub revocation: CheckStatus,
    pub egress_denial: CheckStatus,
    pub artifact_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceEvidence {
    pub origin: EvidenceOrigin,
    pub owner_happy_path: CheckStatus,
    pub visitor_happy_path: CheckStatus,
    pub correction_path: CheckStatus,
    pub failure_recovery_path: CheckStatus,
    pub revoke_deny_path: CheckStatus,
    pub artifact_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HumanDimensions {
    pub voice_similarity: RecordStatus,
    pub voice_naturalness: RecordStatus,
    pub appearance_plausibility: RecordStatus,
    pub persona_similarity: RecordStatus,
    pub conversation_naturalness: RecordStatus,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HumanEvaluationEvidence {
    pub origin: EvidenceOrigin,
    pub rubric_version: String,
    pub reviewer_count: u32,
    pub dimensions: HumanDimensions,
    pub usable_for_continuation: CheckStatus,
    pub artifact_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct KnownLimitationsEvidence {
    pub review_status: CheckStatus,
    pub document_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Rt0ExitEvidence {
    pub schema_version: String,
    pub candidate_sha: String,
    pub release_spec_sha256: String,
    pub golden_report_sha256: String,
    pub provider_state_sha256: String,
    pub live_provider_probe_sha256: String,
    pub conversation_attempt_sha256: String,
    pub bound_session_aggregate_sha256: String,
    pub automated: AutomatedEvidence,
    pub conversations: ConversationPairEvidence,
    pub acceptance: AcceptanceEvidence,
    pub quality: QualityEvidence,
    pub cost: CostEvidence,
    pub privacy_permissions: PrivacyPermissionEvidence,
    pub human_evaluation: HumanEvaluationEvidence,
    pub known_limitations: KnownLimitationsEvidence,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Rt0ExitFailureCode {
    GoldenSetNotPassed,
    CiNotPassed,
    E2eNotPassed,
    OwnerConversationNotReal,
    OwnerConversationIncomplete,
    OwnerInterruptionNotExercised,
    VisitorConversationNotReal,
    VisitorConversationIncomplete,
    AcceptanceEvidenceNotReal,
    AcceptanceMatrixIncomplete,
    QualityEvidenceNotReal,
    TextLatencyExceeded,
    AudioLatencyExceeded,
    InterruptionLatencyExceeded,
    VideoLatencyExceeded,
    AvSyncExceeded,
    ReconnectLatencyExceeded,
    CostEvidenceNotReal,
    CostNotMeasured,
    PrivacyEvidenceNotReal,
    PermissionSuiteNotPassed,
    PrivateContextLeakageAccepted,
    FalseOwnerAttributionAccepted,
    RevocationNotVerified,
    EgressDenialNotVerified,
    HumanEvaluationNotReal,
    HumanEvaluationIncomplete,
    HumanEvaluationNotUsable,
    KnownLimitationsNotReviewed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Rt0ExitReport {
    pub schema_version: String,
    pub candidate_sha: String,
    pub release_spec_sha256: String,
    pub provider_state_sha256: String,
    pub golden_report_sha256: String,
    pub live_provider_probe_sha256: String,
    pub conversation_attempt_sha256: String,
    pub bound_session_aggregate_sha256: String,
    pub exit_evidence_input_sha256: String,
    pub ready: bool,
    pub failures: Vec<Rt0ExitFailureCode>,
    pub estimated_cost_per_minute_microunits: Option<u64>,
    pub provider_charge_per_minute_microunits: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Rt0ExitEvidenceError {
    UnsupportedExitEvidenceSchema,
    InvalidCandidateSha,
    InvalidDigest,
    CandidateShaMismatch,
    ReleaseSpecDigestMismatch,
    GoldenReportDigestMismatch,
    ProviderStateDigestMismatch,
    ProviderStateMismatch,
    LiveProviderProbeDigestMismatch,
    LiveProviderProbeInvalid,
    LiveProviderProbeCandidateMismatch,
    LiveProviderProbeProviderStateMismatch,
    RuntimeEvidenceDigestMismatch,
    RuntimeEvidenceInvalid,
    RuntimeEvidenceCandidateMismatch,
    RuntimeEvidenceProviderStateMismatch,
    GoldenEvidenceInvalid,
    GoldenReportRecomputeMismatch,
    GoldenReportInvalid,
    ParticipantRoleMismatch,
    InvalidLatencyDistribution,
    InvalidHumanRubric,
    InvalidArtifactDigest,
}

/// Evaluates whether one exact-candidate RT0 evidence bundle satisfies the `ReleaseSpec` exit gate.
///
/// Structural or stale evidence is returned as `Rt0ExitEvidenceError`. Valid-but-insufficient
/// evidence produces a report with `ready = false` and stable failure codes.
///
/// # Errors
/// Returns `Rt0ExitEvidenceError` when evidence is malformed, stale, cross-candidate, or internally
/// inconsistent.
pub fn evaluate_rt0_exit_evidence(
    evidence: &Rt0ExitEvidence,
    golden_report: &BoundGoldenReport,
    context: Rt0ExitVerificationContext<'_>,
) -> Result<Rt0ExitReport, Rt0ExitEvidenceError> {
    validate_structure(evidence, golden_report, context)?;
    let mut failures = Vec::new();

    if golden_report.golden.failed != 0 || golden_report.golden.passed != golden_report.golden.total
    {
        failures.push(Rt0ExitFailureCode::GoldenSetNotPassed);
    }
    if evidence.automated.ci.status != CheckStatus::Passed {
        failures.push(Rt0ExitFailureCode::CiNotPassed);
    }
    if evidence.automated.e2e.status != CheckStatus::Passed {
        failures.push(Rt0ExitFailureCode::E2eNotPassed);
    }
    evaluate_conversations(&evidence.conversations, &mut failures);
    evaluate_acceptance(&evidence.acceptance, &mut failures);
    evaluate_quality(&evidence.quality, &mut failures);
    evaluate_cost(&evidence.cost, &mut failures);
    evaluate_privacy(&evidence.privacy_permissions, &mut failures);
    evaluate_human(&evidence.human_evaluation, &mut failures);
    if evidence.known_limitations.review_status != CheckStatus::Passed {
        failures.push(Rt0ExitFailureCode::KnownLimitationsNotReviewed);
    }

    let estimated_cost_per_minute_microunits = cost_per_minute(
        evidence.cost.estimated_cost_microunits,
        evidence.cost.measured_duration_millis,
    );
    let provider_charge_per_minute_microunits = cost_per_minute(
        evidence.cost.provider_charge_microunits,
        evidence.cost.measured_duration_millis,
    );
    Ok(Rt0ExitReport {
        schema_version: RT0_EXIT_REPORT_SCHEMA.into(),
        candidate_sha: evidence.candidate_sha.clone(),
        release_spec_sha256: evidence.release_spec_sha256.clone(),
        provider_state_sha256: evidence.provider_state_sha256.clone(),
        golden_report_sha256: evidence.golden_report_sha256.clone(),
        live_provider_probe_sha256: evidence.live_provider_probe_sha256.clone(),
        conversation_attempt_sha256: evidence.conversation_attempt_sha256.clone(),
        bound_session_aggregate_sha256: evidence.bound_session_aggregate_sha256.clone(),
        exit_evidence_input_sha256: sha256_hex(context.exit_evidence_bytes),
        ready: failures.is_empty(),
        failures,
        estimated_cost_per_minute_microunits,
        provider_charge_per_minute_microunits,
    })
}

fn validate_structure(
    evidence: &Rt0ExitEvidence,
    golden_report: &BoundGoldenReport,
    context: Rt0ExitVerificationContext<'_>,
) -> Result<(), Rt0ExitEvidenceError> {
    if evidence.schema_version != RT0_EXIT_EVIDENCE_SCHEMA {
        return Err(Rt0ExitEvidenceError::UnsupportedExitEvidenceSchema);
    }
    if !valid_git_sha(context.exact_candidate_sha) || !valid_git_sha(&evidence.candidate_sha) {
        return Err(Rt0ExitEvidenceError::InvalidCandidateSha);
    }
    for digest in [
        &evidence.release_spec_sha256,
        &evidence.golden_report_sha256,
        &evidence.provider_state_sha256,
        &evidence.live_provider_probe_sha256,
        &evidence.conversation_attempt_sha256,
        &evidence.bound_session_aggregate_sha256,
    ] {
        if !valid_sha256(digest) {
            return Err(Rt0ExitEvidenceError::InvalidDigest);
        }
    }
    if evidence.candidate_sha != context.exact_candidate_sha
        || golden_report.binding.candidate_sha != context.exact_candidate_sha
    {
        return Err(Rt0ExitEvidenceError::CandidateShaMismatch);
    }
    let release_spec_digest = sha256_hex(context.release_spec_bytes);
    if evidence.release_spec_sha256 != release_spec_digest
        || golden_report.binding.release_spec_sha256 != release_spec_digest
    {
        return Err(Rt0ExitEvidenceError::ReleaseSpecDigestMismatch);
    }
    if evidence.golden_report_sha256 != sha256_hex(context.golden_report_bytes) {
        return Err(Rt0ExitEvidenceError::GoldenReportDigestMismatch);
    }
    if evidence.live_provider_probe_sha256 != sha256_hex(context.live_provider_probe_bytes) {
        return Err(Rt0ExitEvidenceError::LiveProviderProbeDigestMismatch);
    }
    let provider_state_digest = sha256_hex(context.provider_state_bytes);
    if evidence.provider_state_sha256 != provider_state_digest
        || golden_report.binding.provider_state_sha256 != provider_state_digest
    {
        return Err(Rt0ExitEvidenceError::ProviderStateDigestMismatch);
    }
    if context.provider_state.schema_version != RT0_PROVIDER_STATE_SCHEMA
        || validate_provider_state(&context.provider_state.providers).is_err()
        || golden_report.provider_state != *context.provider_state
    {
        return Err(Rt0ExitEvidenceError::ProviderStateMismatch);
    }
    validate_live_provider_probe(
        context.live_provider_probe,
        context.exact_candidate_sha,
        &provider_state_digest,
    )
    .map_err(|error| match error {
        LiveProviderProbeValidationError::Invalid => Rt0ExitEvidenceError::LiveProviderProbeInvalid,
        LiveProviderProbeValidationError::CandidateMismatch => {
            Rt0ExitEvidenceError::LiveProviderProbeCandidateMismatch
        }
        LiveProviderProbeValidationError::ProviderStateMismatch => {
            Rt0ExitEvidenceError::LiveProviderProbeProviderStateMismatch
        }
    })?;
    validate_runtime_evidence(evidence, context, &provider_state_digest)?;
    validate_golden_report(golden_report)?;
    let required_suite: GoldenSuite = serde_json::from_slice(RT0_REQUIRED_GOLDEN_SUITE_BYTES)
        .map_err(|_| Rt0ExitEvidenceError::GoldenEvidenceInvalid)?;
    let recomputed = evaluate_bound_golden_suite(
        &required_suite,
        context.golden_evidence_bundle,
        EvidenceVerificationContext {
            suite_bytes: RT0_REQUIRED_GOLDEN_SUITE_BYTES,
            release_spec_bytes: context.release_spec_bytes,
            provider_state: context.provider_state,
            provider_state_bytes: context.provider_state_bytes,
            evidence_bytes: context.golden_evidence_bytes,
            exact_candidate_sha: context.exact_candidate_sha,
        },
    )
    .map_err(|_| Rt0ExitEvidenceError::GoldenEvidenceInvalid)?;
    if recomputed != *golden_report {
        return Err(Rt0ExitEvidenceError::GoldenReportRecomputeMismatch);
    }
    validate_artifact_digests(evidence)?;
    if evidence.conversations.owner.role != ParticipantRole::Owner
        || evidence.conversations.visitor.role != ParticipantRole::Visitor
    {
        return Err(Rt0ExitEvidenceError::ParticipantRoleMismatch);
    }
    if evidence.human_evaluation.rubric_version.trim().is_empty() {
        return Err(Rt0ExitEvidenceError::InvalidHumanRubric);
    }
    Ok(())
}

fn validate_golden_report(report: &BoundGoldenReport) -> Result<(), Rt0ExitEvidenceError> {
    if report.binding.schema_version != RT0_EVIDENCE_BINDING_SCHEMA
        || report.provider_state.schema_version != RT0_PROVIDER_STATE_SCHEMA
        || report.golden.schema_version != RT0_GOLDEN_SCHEMA
        || !valid_sha256(&report.evidence_input_sha256)
        || !valid_sha256(&report.binding.release_spec_sha256)
        || !valid_sha256(&report.binding.suite_sha256)
        || !valid_sha256(&report.binding.provider_state_sha256)
        || report.golden.total == 0
        || report.golden.total != report.golden.cases.len()
        || report.golden.passed.checked_add(report.golden.failed) != Some(report.golden.total)
        || validate_provider_state(&report.provider_state.providers).is_err()
    {
        return Err(Rt0ExitEvidenceError::GoldenReportInvalid);
    }
    let mut ids = HashSet::new();
    for case in &report.golden.cases {
        if case.case_id.trim().is_empty()
            || !ids.insert(case.case_id.as_str())
            || case.passed != case.failures.is_empty()
        {
            return Err(Rt0ExitEvidenceError::GoldenReportInvalid);
        }
    }
    Ok(())
}

fn validate_artifact_digests(evidence: &Rt0ExitEvidence) -> Result<(), Rt0ExitEvidenceError> {
    let digests = [
        evidence.automated.ci.artifact_sha256.as_str(),
        evidence.automated.e2e.artifact_sha256.as_str(),
        evidence.conversations.owner.artifact_sha256.as_str(),
        evidence.conversations.visitor.artifact_sha256.as_str(),
        evidence.acceptance.artifact_sha256.as_str(),
        evidence.quality.artifact_sha256.as_str(),
        evidence.cost.artifact_sha256.as_str(),
        evidence.privacy_permissions.artifact_sha256.as_str(),
        evidence.human_evaluation.artifact_sha256.as_str(),
        evidence.known_limitations.document_sha256.as_str(),
    ];
    if digests.into_iter().all(valid_sha256) {
        Ok(())
    } else {
        Err(Rt0ExitEvidenceError::InvalidArtifactDigest)
    }
}

fn evaluate_conversations(
    evidence: &ConversationPairEvidence,
    failures: &mut Vec<Rt0ExitFailureCode>,
) {
    if evidence.owner.origin != EvidenceOrigin::Real {
        failures.push(Rt0ExitFailureCode::OwnerConversationNotReal);
    }
    if !conversation_complete(&evidence.owner) {
        failures.push(Rt0ExitFailureCode::OwnerConversationIncomplete);
    }
    if evidence.owner.interruption_exercised != CheckStatus::Passed {
        failures.push(Rt0ExitFailureCode::OwnerInterruptionNotExercised);
    }
    if evidence.visitor.origin != EvidenceOrigin::Real {
        failures.push(Rt0ExitFailureCode::VisitorConversationNotReal);
    }
    if !conversation_complete(&evidence.visitor) {
        failures.push(Rt0ExitFailureCode::VisitorConversationIncomplete);
    }
}

fn conversation_complete(evidence: &ConversationEvidence) -> bool {
    evidence.russian == CheckStatus::Passed
        && evidence.voice == CheckStatus::Passed
        && evidence.video == CheckStatus::Passed
        && evidence.completed_turns > 0
}

fn evaluate_acceptance(evidence: &AcceptanceEvidence, failures: &mut Vec<Rt0ExitFailureCode>) {
    if evidence.origin != EvidenceOrigin::Real {
        failures.push(Rt0ExitFailureCode::AcceptanceEvidenceNotReal);
    }
    if evidence.owner_happy_path != CheckStatus::Passed
        || evidence.visitor_happy_path != CheckStatus::Passed
        || evidence.correction_path != CheckStatus::Passed
        || evidence.failure_recovery_path != CheckStatus::Passed
        || evidence.revoke_deny_path != CheckStatus::Passed
    {
        failures.push(Rt0ExitFailureCode::AcceptanceMatrixIncomplete);
    }
}

fn evaluate_quality(evidence: &QualityEvidence, failures: &mut Vec<Rt0ExitFailureCode>) {
    if evidence.origin != EvidenceOrigin::Real {
        failures.push(Rt0ExitFailureCode::QualityEvidenceNotReal);
    }
    if evidence.text_first_meaningful_response.p50 > 1_000
        || evidence.text_first_meaningful_response.p95 > 2_500
    {
        failures.push(Rt0ExitFailureCode::TextLatencyExceeded);
    }
    if evidence.first_meaningful_audio.p50 > 1_500 || evidence.first_meaningful_audio.p95 > 3_000 {
        failures.push(Rt0ExitFailureCode::AudioLatencyExceeded);
    }
    if evidence.interruption_stop.p95 > 500 {
        failures.push(Rt0ExitFailureCode::InterruptionLatencyExceeded);
    }
    if evidence.first_useful_video.p95 > 2_500 {
        failures.push(Rt0ExitFailureCode::VideoLatencyExceeded);
    }
    if evidence.av_sync_absolute_offset.p95 > 120 {
        failures.push(Rt0ExitFailureCode::AvSyncExceeded);
    }
    if evidence.recoverable_reconnect.p95 > 5_000 {
        failures.push(Rt0ExitFailureCode::ReconnectLatencyExceeded);
    }
}

fn evaluate_cost(evidence: &CostEvidence, failures: &mut Vec<Rt0ExitFailureCode>) {
    if evidence.origin != EvidenceOrigin::Real {
        failures.push(Rt0ExitFailureCode::CostEvidenceNotReal);
    }
    if evidence.measured_duration_millis == 0
        || (evidence.estimated_cost_microunits.is_none()
            && evidence.provider_charge_microunits.is_none())
    {
        failures.push(Rt0ExitFailureCode::CostNotMeasured);
    }
}

fn evaluate_privacy(evidence: &PrivacyPermissionEvidence, failures: &mut Vec<Rt0ExitFailureCode>) {
    if evidence.origin != EvidenceOrigin::Real {
        failures.push(Rt0ExitFailureCode::PrivacyEvidenceNotReal);
    }
    if evidence.permission_suite != CheckStatus::Passed {
        failures.push(Rt0ExitFailureCode::PermissionSuiteNotPassed);
    }
    if evidence.accepted_private_context_leakage != 0 {
        failures.push(Rt0ExitFailureCode::PrivateContextLeakageAccepted);
    }
    if evidence.accepted_false_owner_attribution != 0 {
        failures.push(Rt0ExitFailureCode::FalseOwnerAttributionAccepted);
    }
    if evidence.revocation != CheckStatus::Passed {
        failures.push(Rt0ExitFailureCode::RevocationNotVerified);
    }
    if evidence.egress_denial != CheckStatus::Passed {
        failures.push(Rt0ExitFailureCode::EgressDenialNotVerified);
    }
}

fn evaluate_human(evidence: &HumanEvaluationEvidence, failures: &mut Vec<Rt0ExitFailureCode>) {
    if evidence.origin != EvidenceOrigin::Real {
        failures.push(Rt0ExitFailureCode::HumanEvaluationNotReal);
    }
    if evidence.reviewer_count == 0
        || evidence.dimensions.voice_similarity != RecordStatus::Recorded
        || evidence.dimensions.voice_naturalness != RecordStatus::Recorded
        || evidence.dimensions.appearance_plausibility != RecordStatus::Recorded
        || evidence.dimensions.persona_similarity != RecordStatus::Recorded
        || evidence.dimensions.conversation_naturalness != RecordStatus::Recorded
    {
        failures.push(Rt0ExitFailureCode::HumanEvaluationIncomplete);
    }
    if evidence.usable_for_continuation != CheckStatus::Passed {
        failures.push(Rt0ExitFailureCode::HumanEvaluationNotUsable);
    }
}

fn cost_per_minute(cost: Option<u64>, measured_duration_millis: u64) -> Option<u64> {
    let cost = cost?;
    if measured_duration_millis == 0 {
        return None;
    }
    let numerator = u128::from(cost).checked_mul(60_000)?;
    let value = numerator / u128::from(measured_duration_millis);
    u64::try_from(value).ok()
}
