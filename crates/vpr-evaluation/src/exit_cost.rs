use std::collections::HashSet;

use crate::{CostEvidence, EvidenceOrigin, ProviderRole, Rt0ExitFailureCode};

pub(crate) fn evaluate_cost(evidence: &CostEvidence, failures: &mut Vec<Rt0ExitFailureCode>) {
    if evidence.origin != EvidenceOrigin::Real {
        failures.push(Rt0ExitFailureCode::CostEvidenceNotReal);
    }
    let estimate_complete = signal_complete(
        evidence.estimated_cost_microunits,
        &evidence.estimated_cost_covered_provider_roles,
    );
    let charge_complete = signal_complete(
        evidence.provider_charge_microunits,
        &evidence.provider_charge_covered_provider_roles,
    );
    if !estimate_complete && !charge_complete {
        failures.push(Rt0ExitFailureCode::CostProviderCoverageIncomplete);
    }
    if evidence.measured_duration_millis == 0
        || (evidence.estimated_cost_microunits.is_none()
            && evidence.provider_charge_microunits.is_none())
    {
        failures.push(Rt0ExitFailureCode::CostNotMeasured);
    }
}

pub(crate) fn estimated_cost_per_minute(evidence: &CostEvidence) -> Option<u64> {
    cost_per_minute(
        evidence,
        evidence.estimated_cost_microunits,
        &evidence.estimated_cost_covered_provider_roles,
    )
}

pub(crate) fn provider_charge_per_minute(evidence: &CostEvidence) -> Option<u64> {
    cost_per_minute(
        evidence,
        evidence.provider_charge_microunits,
        &evidence.provider_charge_covered_provider_roles,
    )
}

fn cost_per_minute(
    evidence: &CostEvidence,
    cost: Option<u64>,
    covered_provider_roles: &[ProviderRole],
) -> Option<u64> {
    let cost = cost?;
    if evidence.measured_duration_millis == 0 || !coverage_complete(covered_provider_roles) {
        return None;
    }
    let numerator = u128::from(cost).checked_mul(60_000)?;
    let value = numerator / u128::from(evidence.measured_duration_millis);
    u64::try_from(value).ok()
}

fn signal_complete(cost: Option<u64>, covered_provider_roles: &[ProviderRole]) -> bool {
    cost.is_some() && coverage_complete(covered_provider_roles)
}

fn coverage_complete(covered_provider_roles: &[ProviderRole]) -> bool {
    let covered: HashSet<_> = covered_provider_roles.iter().copied().collect();
    covered.len() == 3
        && covered.contains(&ProviderRole::Stt)
        && covered.contains(&ProviderRole::Llm)
        && covered.contains(&ProviderRole::Avatar)
}
