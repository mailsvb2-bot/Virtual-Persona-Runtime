use crate::{
    BoundGoldenReport, EvidenceBindingError, EvidenceVerificationContext, GoldenActor,
    GoldenEvidenceBundle, GoldenExpectation, GoldenSuite, evaluate_bound_golden_suite,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerGoldenError {
    SuiteNotOwnerSpecific,
    ReportMismatch,
    Binding(EvidenceBindingError),
}

impl From<EvidenceBindingError> for OwnerGoldenError {
    fn from(value: EvidenceBindingError) -> Self {
        Self::Binding(value)
    }
}

/// Evaluates a private owner-specific Golden Set while retaining the same exact-candidate,
/// release-spec and provider-state binding as the public RT0 baseline.
///
/// Owner-specific suite bytes are deliberately runtime inputs and must never be embedded in the
/// binary or committed to the repository.
///
/// # Errors
/// Rejects suites that do not identify themselves as private owner suites, are too small to be
/// meaningful, or contain no owner-specific content/attribution checks. Binding failures are
/// propagated without weakening the normal Golden evaluator.
pub fn evaluate_bound_owner_golden_suite(
    suite: &GoldenSuite,
    bundle: &GoldenEvidenceBundle,
    context: EvidenceVerificationContext<'_>,
) -> Result<BoundGoldenReport, OwnerGoldenError> {
    validate_owner_suite(suite)?;
    evaluate_bound_golden_suite(suite, bundle, context).map_err(Into::into)
}

pub(crate) fn verify_private_owner_golden(
    context: crate::Rt0ExitVerificationContext<'_>,
) -> Result<(), OwnerGoldenError> {
    let recomputed = evaluate_bound_owner_golden_suite(
        context.owner_golden_suite,
        context.owner_golden_evidence_bundle,
        EvidenceVerificationContext {
            suite_bytes: context.owner_golden_suite_bytes,
            release_spec_bytes: context.release_spec_bytes,
            provider_state: context.provider_state,
            provider_state_bytes: context.provider_state_bytes,
            evidence_bytes: context.owner_golden_evidence_bytes,
            exact_candidate_sha: context.exact_candidate_sha,
        },
    )?;
    if recomputed != *context.owner_golden_report {
        return Err(OwnerGoldenError::ReportMismatch);
    }
    Ok(())
}

fn validate_owner_suite(suite: &GoldenSuite) -> Result<(), OwnerGoldenError> {
    if !suite.suite_id.starts_with("rt0.owner.")
        || suite.cases.len() < 3
        || !suite.cases.iter().any(|case| case.actor == GoldenActor::Owner)
    {
        return Err(OwnerGoldenError::SuiteNotOwnerSpecific);
    }

    let has_private_fidelity_probe = suite.cases.iter().any(|case| {
        case.actor == GoldenActor::Owner
            && case.expectations.iter().any(|expectation| {
                matches!(
                    expectation,
                    GoldenExpectation::ResponseContainsAll { tokens } if !tokens.is_empty()
                )
            })
    });
    let has_verified_owner_attribution = suite.cases.iter().any(|case| {
        case.actor == GoldenActor::Owner
            && case.expectations.iter().any(|expectation| {
                matches!(
                    expectation,
                    GoldenExpectation::OwnerAttribution { eligible: true }
                )
            })
    });

    if !has_private_fidelity_probe || !has_verified_owner_attribution {
        return Err(OwnerGoldenError::SuiteNotOwnerSpecific);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suite(id: &str, cases: serde_json::Value) -> GoldenSuite {
        serde_json::from_value(serde_json::json!({
            "schema_version": "rt0-golden-0.1",
            "suite_id": id,
            "cases": cases,
        }))
        .unwrap()
    }

    #[test]
    fn private_owner_suite_requires_fidelity_and_verified_attribution_probes() {
        let valid = suite(
            "rt0.owner.private-v1",
            serde_json::json!([
                {
                    "id":"owner.fact",
                    "actor":"owner",
                    "prompt_ru":"Закрытый owner-specific вопрос",
                    "expectations":[{"kind":"response_contains_all","tokens":["PRIVATE_EXPECTED_TOKEN"]}]
                },
                {
                    "id":"owner.attribution",
                    "actor":"owner",
                    "prompt_ru":"Проверка подтверждённого мнения",
                    "expectations":[{"kind":"owner_attribution","eligible":true}]
                },
                {
                    "id":"owner.stability",
                    "actor":"owner",
                    "prompt_ru":"Проверка идентичности",
                    "expectations":[{"kind":"persona_identity_stable"}]
                }
            ]),
        );
        assert_eq!(validate_owner_suite(&valid), Ok(()));

        let public_name = suite("rt0.minimum.synthetic-persona", serde_json::json!([]));
        assert_eq!(
            validate_owner_suite(&public_name),
            Err(OwnerGoldenError::SuiteNotOwnerSpecific)
        );
    }
}
