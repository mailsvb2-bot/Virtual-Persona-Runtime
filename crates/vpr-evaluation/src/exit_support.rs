use serde::Serialize;
use serde_json::Value;

use crate::binding::valid_sha256;
use crate::{
    BoundGoldenReport, Rt0ExitEvidence, Rt0ExitEvidenceError, Rt0ExitReport,
    Rt0ExitVerificationContext, evaluate_rt0_exit_evidence, sha256_hex,
};

#[derive(Debug, Clone, Copy)]
pub struct Rt0ExitSupportingArtifacts<'a> {
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

/// Validates that every declared supporting-evidence digest is bound to the exact supplied bytes.
///
/// # Errors
/// Returns `Rt0ExitEvidenceError::InvalidArtifactDigest` when a digest is malformed or detached
/// from the corresponding presented artifact bytes.
pub fn validate_rt0_exit_supporting_artifacts(
    evidence: &Rt0ExitEvidence,
    artifacts: Rt0ExitSupportingArtifacts<'_>,
) -> Result<(), Rt0ExitEvidenceError> {
    let bound = [
        (evidence.automated.ci.artifact_sha256.as_str(), artifacts.ci),
        (
            evidence.automated.e2e.artifact_sha256.as_str(),
            artifacts.e2e,
        ),
        (
            evidence.conversations.owner.artifact_sha256.as_str(),
            artifacts.owner_conversation,
        ),
        (
            evidence.conversations.visitor.artifact_sha256.as_str(),
            artifacts.visitor_conversation,
        ),
        (
            evidence.acceptance.artifact_sha256.as_str(),
            artifacts.acceptance,
        ),
        (evidence.quality.artifact_sha256.as_str(), artifacts.quality),
        (evidence.cost.artifact_sha256.as_str(), artifacts.cost),
        (
            evidence.privacy_permissions.artifact_sha256.as_str(),
            artifacts.privacy_permissions,
        ),
        (
            evidence.human_evaluation.artifact_sha256.as_str(),
            artifacts.human_evaluation,
        ),
        (
            evidence.known_limitations.document_sha256.as_str(),
            artifacts.known_limitations,
        ),
    ];
    if !bound.into_iter().all(|(digest, bytes)| {
        let actual = sha256_hex(bytes);
        valid_sha256(digest) && digest == actual.as_str()
    }) {
        return Err(Rt0ExitEvidenceError::InvalidArtifactDigest);
    }

    let claims = [
        (artifacts.ci, claim_without_digest(&evidence.automated.ci)?),
        (artifacts.e2e, claim_without_digest(&evidence.automated.e2e)?),
        (
            artifacts.owner_conversation,
            claim_without_digest(&evidence.conversations.owner)?,
        ),
        (
            artifacts.visitor_conversation,
            claim_without_digest(&evidence.conversations.visitor)?,
        ),
        (artifacts.acceptance, claim_without_digest(&evidence.acceptance)?),
        (artifacts.quality, claim_without_digest(&evidence.quality)?),
        (artifacts.cost, claim_without_digest(&evidence.cost)?),
        (
            artifacts.privacy_permissions,
            claim_without_digest(&evidence.privacy_permissions)?,
        ),
        (
            artifacts.human_evaluation,
            claim_without_digest(&evidence.human_evaluation)?,
        ),
    ];
    if claims.into_iter().all(|(bytes, expected)| {
        serde_json::from_slice::<Value>(bytes).is_ok_and(|actual| actual == expected)
    }) {
        Ok(())
    } else {
        Err(Rt0ExitEvidenceError::InvalidArtifactDigest)
    }
}

fn claim_without_digest<T: Serialize>(claim: &T) -> Result<Value, Rt0ExitEvidenceError> {
    let mut value =
        serde_json::to_value(claim).map_err(|_| Rt0ExitEvidenceError::InvalidArtifactDigest)?;
    let Some(object) = value.as_object_mut() else {
        return Err(Rt0ExitEvidenceError::InvalidArtifactDigest);
    };
    if object.remove("artifact_sha256").is_none() {
        return Err(Rt0ExitEvidenceError::InvalidArtifactDigest);
    }
    Ok(value)
}

/// Evaluates RT0 exit evidence only after binding every supporting artifact digest to exact bytes.
///
/// This is the release-verification entry point used by `vpr-rt0-exit-evidence`. The lower-level
/// `evaluate_rt0_exit_evidence` remains available for deterministic claim evaluation in tests and
/// tooling that do not have filesystem artifact bytes.
///
/// # Errors
/// Returns `Rt0ExitEvidenceError` when supporting artifacts or the remaining evidence bundle are
/// malformed, stale, cross-candidate, or internally inconsistent.
pub fn evaluate_verified_rt0_exit_evidence(
    evidence: &Rt0ExitEvidence,
    golden_report: &BoundGoldenReport,
    context: Rt0ExitVerificationContext<'_>,
    supporting_artifacts: Rt0ExitSupportingArtifacts<'_>,
) -> Result<Rt0ExitReport, Rt0ExitEvidenceError> {
    validate_rt0_exit_supporting_artifacts(evidence, supporting_artifacts)?;
    evaluate_rt0_exit_evidence(evidence, golden_report, context)
}
