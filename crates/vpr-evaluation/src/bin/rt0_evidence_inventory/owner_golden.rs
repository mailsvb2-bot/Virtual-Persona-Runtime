use super::ParsedBindingArtifacts;
use vpr_evaluation::{EvidenceVerificationContext, evaluate_bound_owner_golden_suite};

pub(super) fn recompute_owner_golden_report(
    parsed: &ParsedBindingArtifacts,
    candidate_sha: &str,
) -> Option<bool> {
    let suite = parsed.owner_golden_suite.as_ref()?;
    let report = parsed.owner_golden_report.as_ref()?;
    let bundle = parsed.owner_golden_evidence.as_ref()?;
    let provider_state = parsed.provider.as_ref()?;
    let provider_state_bytes = parsed.provider_state_bytes.as_deref()?;
    let suite_bytes = parsed.owner_golden_suite_bytes.as_deref()?;
    let evidence_bytes = parsed.owner_golden_evidence_bytes.as_deref()?;
    let release_spec_bytes = parsed.release_spec_bytes.as_deref()?;
    Some(
        evaluate_bound_owner_golden_suite(
            suite,
            bundle,
            EvidenceVerificationContext {
                suite_bytes,
                release_spec_bytes,
                provider_state,
                provider_state_bytes,
                evidence_bytes,
                exact_candidate_sha: candidate_sha,
            },
        )
        .is_ok_and(|recomputed| recomputed == *report),
    )
}
