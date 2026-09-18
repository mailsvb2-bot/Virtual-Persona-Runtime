use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::exit_validation::{
    validate_bound_session_aggregate, validate_browser_quality_binding,
    validate_rt0_conversation_attempt_artifact,
};
use crate::live_provider::validate_live_provider_probe;
use crate::{
    AutomatedEvidence, BoundGoldenReport, BoundLabSessionEvidenceAggregate,
    ConversationPairEvidence, KnownLimitationsEvidence, LiveProviderProbeReceipt,
    ProviderStateManifest, RT0_EVIDENCE_BINDING_SCHEMA, RT0_EXIT_EVIDENCE_SCHEMA, Rt0ExitEvidence,
    Rt0SupportingPreflightArtifacts, preflight_rt0_supporting_artifacts, sha256_hex,
};

#[derive(Debug, Clone, Copy)]
pub struct Rt0ExitAssemblyInputs<'a> {
    pub supporting: Rt0SupportingPreflightArtifacts<'a>,
    pub bound_golden_report_bytes: &'a [u8],
    pub provider_state_bytes: &'a [u8],
    pub live_provider_probe_bytes: &'a [u8],
    pub conversation_attempt_bytes: &'a [u8],
    pub bound_session_aggregate_bytes: &'a [u8],
    pub release_spec_bytes: &'a [u8],
    pub exact_candidate_sha: &'a str,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Rt0ExitAssemblyError {
    SupportingEvidenceInvalid,
    GoldenReportInvalid,
    GoldenCandidateMismatch,
    GoldenReleaseSpecMismatch,
    GoldenProviderStateMismatch,
    LiveProviderProbeInvalid,
    ConversationAttemptInvalid,
    ConversationCandidateMismatch,
    ConversationProviderStateMismatch,
    BoundSessionAggregateInvalid,
    SessionCandidateMismatch,
    SessionProviderStateMismatch,
    BrowserQualityMismatch,
    SupportingProjectionInvalid,
}

/// Assembles one RT0 exit-evidence manifest exclusively from exact already-recorded artifacts.
///
/// The assembler never evaluates release readiness and never changes substantive pass/fail values.
/// Supporting claims are copied from exact candidate/provider-bound artifact bytes after the
/// non-promoting supporting-evidence preflight succeeds.
///
/// # Errors
/// Returns a stable assembly error when supporting evidence is invalid, when any core artifact is
/// malformed or stale for the exact candidate/provider/spec binding, or when a supporting JSON
/// projection cannot be converted into the canonical exit-evidence claim type.
pub fn assemble_rt0_exit_evidence(
    inputs: Rt0ExitAssemblyInputs<'_>,
) -> Result<Rt0ExitEvidence, Rt0ExitAssemblyError> {
    let supporting = preflight_rt0_supporting_artifacts(
        inputs.supporting,
        inputs.provider_state_bytes,
        inputs.exact_candidate_sha,
    )
    .map_err(|_| Rt0ExitAssemblyError::SupportingEvidenceInvalid)?;
    let session = validate_core_artifacts(&inputs, &supporting.provider_state_sha256)?;

    let automated = AutomatedEvidence {
        ci: projected_claim(
            inputs.supporting.ci,
            &supporting.artifact_digests.ci,
            ProjectionBinding::Candidate,
        )?,
        e2e: projected_claim(
            inputs.supporting.e2e,
            &supporting.artifact_digests.e2e,
            ProjectionBinding::Candidate,
        )?,
    };
    let conversations = ConversationPairEvidence {
        owner: projected_claim(
            inputs.supporting.owner_conversation,
            &supporting.artifact_digests.owner_conversation,
            ProjectionBinding::CandidateAndProvider,
        )?,
        visitor: projected_claim(
            inputs.supporting.visitor_conversation,
            &supporting.artifact_digests.visitor_conversation,
            ProjectionBinding::CandidateAndProvider,
        )?,
    };

    let evidence = Rt0ExitEvidence {
        schema_version: RT0_EXIT_EVIDENCE_SCHEMA.into(),
        candidate_sha: inputs.exact_candidate_sha.into(),
        release_spec_sha256: sha256_hex(inputs.release_spec_bytes),
        golden_report_sha256: sha256_hex(inputs.bound_golden_report_bytes),
        provider_state_sha256: supporting.provider_state_sha256,
        live_provider_probe_sha256: sha256_hex(inputs.live_provider_probe_bytes),
        conversation_attempt_sha256: sha256_hex(inputs.conversation_attempt_bytes),
        bound_session_aggregate_sha256: sha256_hex(inputs.bound_session_aggregate_bytes),
        automated,
        conversations,
        acceptance: projected_claim(
            inputs.supporting.acceptance,
            &supporting.artifact_digests.acceptance,
            ProjectionBinding::CandidateAndProvider,
        )?,
        quality: projected_claim(
            inputs.supporting.quality,
            &supporting.artifact_digests.quality,
            ProjectionBinding::CandidateAndProvider,
        )?,
        cost: projected_claim(
            inputs.supporting.cost,
            &supporting.artifact_digests.cost,
            ProjectionBinding::CandidateAndProvider,
        )?,
        privacy_permissions: projected_claim(
            inputs.supporting.privacy_permissions,
            &supporting.artifact_digests.privacy_permissions,
            ProjectionBinding::CandidateAndProvider,
        )?,
        human_evaluation: projected_claim(
            inputs.supporting.human_evaluation,
            &supporting.artifact_digests.human_evaluation,
            ProjectionBinding::CandidateAndProvider,
        )?,
        known_limitations: KnownLimitationsEvidence {
            review_status: supporting.known_limitations_review_status,
            document_sha256: supporting.artifact_digests.known_limitations,
        },
    };
    validate_browser_quality_binding(&evidence, &session.aggregate)
        .map_err(|_| Rt0ExitAssemblyError::BrowserQualityMismatch)?;
    Ok(evidence)
}

fn validate_core_artifacts(
    inputs: &Rt0ExitAssemblyInputs<'_>,
    provider_state_sha256: &str,
) -> Result<BoundLabSessionEvidenceAggregate, Rt0ExitAssemblyError> {
    let provider_state: ProviderStateManifest = serde_json::from_slice(inputs.provider_state_bytes)
        .map_err(|_| Rt0ExitAssemblyError::GoldenProviderStateMismatch)?;
    let golden: BoundGoldenReport = serde_json::from_slice(inputs.bound_golden_report_bytes)
        .map_err(|_| Rt0ExitAssemblyError::GoldenReportInvalid)?;
    if golden.binding.schema_version != RT0_EVIDENCE_BINDING_SCHEMA {
        return Err(Rt0ExitAssemblyError::GoldenReportInvalid);
    }
    if golden.binding.candidate_sha != inputs.exact_candidate_sha {
        return Err(Rt0ExitAssemblyError::GoldenCandidateMismatch);
    }
    if golden.binding.release_spec_sha256 != sha256_hex(inputs.release_spec_bytes) {
        return Err(Rt0ExitAssemblyError::GoldenReleaseSpecMismatch);
    }
    if golden.binding.provider_state_sha256 != provider_state_sha256
        || golden.provider_state != provider_state
    {
        return Err(Rt0ExitAssemblyError::GoldenProviderStateMismatch);
    }

    let probe: LiveProviderProbeReceipt = serde_json::from_slice(inputs.live_provider_probe_bytes)
        .map_err(|_| Rt0ExitAssemblyError::LiveProviderProbeInvalid)?;
    validate_live_provider_probe(&probe, inputs.exact_candidate_sha, provider_state_sha256)
        .map_err(|_| Rt0ExitAssemblyError::LiveProviderProbeInvalid)?;
    validate_rt0_conversation_attempt_artifact(
        inputs.conversation_attempt_bytes,
        inputs.exact_candidate_sha,
        provider_state_sha256,
    )
    .map_err(|error| match error {
        crate::Rt0ExitEvidenceError::RuntimeEvidenceCandidateMismatch => {
            Rt0ExitAssemblyError::ConversationCandidateMismatch
        }
        crate::Rt0ExitEvidenceError::RuntimeEvidenceProviderStateMismatch => {
            Rt0ExitAssemblyError::ConversationProviderStateMismatch
        }
        _ => Rt0ExitAssemblyError::ConversationAttemptInvalid,
    })?;

    let session: BoundLabSessionEvidenceAggregate =
        serde_json::from_slice(inputs.bound_session_aggregate_bytes)
            .map_err(|_| Rt0ExitAssemblyError::BoundSessionAggregateInvalid)?;
    validate_bound_session_aggregate(&session, inputs.exact_candidate_sha, provider_state_sha256)
        .map_err(|error| match error {
            crate::Rt0ExitEvidenceError::RuntimeEvidenceCandidateMismatch => {
                Rt0ExitAssemblyError::SessionCandidateMismatch
            }
            crate::Rt0ExitEvidenceError::RuntimeEvidenceProviderStateMismatch => {
                Rt0ExitAssemblyError::SessionProviderStateMismatch
            }
            _ => Rt0ExitAssemblyError::BoundSessionAggregateInvalid,
        })?;
    Ok(session)
}

#[derive(Debug, Clone, Copy)]
enum ProjectionBinding {
    Candidate,
    CandidateAndProvider,
}

fn projected_claim<T: DeserializeOwned>(
    bytes: &[u8],
    artifact_sha256: &str,
    binding: ProjectionBinding,
) -> Result<T, Rt0ExitAssemblyError> {
    let mut value: Value = serde_json::from_slice(bytes)
        .map_err(|_| Rt0ExitAssemblyError::SupportingProjectionInvalid)?;
    let object = value
        .as_object_mut()
        .ok_or(Rt0ExitAssemblyError::SupportingProjectionInvalid)?;
    if object.remove("candidate_sha").is_none() {
        return Err(Rt0ExitAssemblyError::SupportingProjectionInvalid);
    }
    if matches!(binding, ProjectionBinding::CandidateAndProvider)
        && object.remove("provider_state_sha256").is_none()
    {
        return Err(Rt0ExitAssemblyError::SupportingProjectionInvalid);
    }
    object.insert(
        "artifact_sha256".into(),
        Value::String(artifact_sha256.to_owned()),
    );
    serde_json::from_value(value).map_err(|_| Rt0ExitAssemblyError::SupportingProjectionInvalid)
}
