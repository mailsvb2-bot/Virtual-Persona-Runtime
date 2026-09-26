use std::collections::HashSet;

use crate::{CostEvidence, EvidenceOrigin, ProviderRole, Rt0ExitFailureCode};

pub(crate) fn evaluate_cost(evidence: &CostEvidence, failures: &mut Vec<Rt0ExitFailureCode>) {
    if evidence.origin != EvidenceOrigin::Real {
        failures.push(Rt0ExitFailureCode::CostEvidenceNotReal);
    }
    if !provider_coverage_complete(evidence) {
        failures.push(Rt0ExitFailureCode::CostProviderCoverageIncomplete);
    }
    if evidence.measured_duration_millis == 0
        || (evidence.estimated_cost_microunits.is_none()
            && evidence.provider_charge_microunits.is_none())
    {
        failures.push(Rt0ExitFailureCode::CostNotMeasured);
    }
}

pub(crate) fn cost_per_minute(evidence: &CostEvidence, cost: Option<u64>) -> Option<u64> {
    let cost = cost?;
    if evidence.measured_duration_millis == 0 || !provider_coverage_complete(evidence) {
        return None;
    }
    let numerator = u128::from(cost).checked_mul(60_000)?;
    let value = numerator / u128::from(evidence.measured_duration_millis);
    u64::try_from(value).ok()
}

fn provider_coverage_complete(evidence: &CostEvidence) -> bool {
    let covered: HashSet<_> = evidence.covered_provider_roles.iter().copied().collect();
    covered.len() == 3
        && covered.contains(&ProviderRole::Stt)
        && covered.contains(&ProviderRole::Llm)
        && covered.contains(&ProviderRole::Avatar)
}
