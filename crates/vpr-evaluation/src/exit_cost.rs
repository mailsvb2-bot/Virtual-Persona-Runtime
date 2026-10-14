use std::collections::HashSet;

use crate::{CostEvidence, EvidenceOrigin, ProviderRole, Rt0ExitFailureCode};

pub(crate) fn evaluate_cost(
    evidence: &CostEvidence,
    requires_tts: bool,
    failures: &mut Vec<Rt0ExitFailureCode>,
) {
    if evidence.origin != EvidenceOrigin::Real {
        failures.push(Rt0ExitFailureCode::CostEvidenceNotReal);
    }
    let estimate_complete = signal_complete(
        evidence.estimated_cost_microunits,
        &evidence.estimated_cost_covered_provider_roles,
        requires_tts,
    );
    let charge_complete = signal_complete(
        evidence.provider_charge_microunits,
        &evidence.provider_charge_covered_provider_roles,
        requires_tts,
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

pub(crate) fn estimated_cost_per_minute(
    evidence: &CostEvidence,
    requires_tts: bool,
) -> Option<u64> {
    cost_per_minute(
        evidence,
        evidence.estimated_cost_microunits,
        &evidence.estimated_cost_covered_provider_roles,
        requires_tts,
    )
}

pub(crate) fn provider_charge_per_minute(
    evidence: &CostEvidence,
    requires_tts: bool,
) -> Option<u64> {
    cost_per_minute(
        evidence,
        evidence.provider_charge_microunits,
        &evidence.provider_charge_covered_provider_roles,
        requires_tts,
    )
}

fn cost_per_minute(
    evidence: &CostEvidence,
    cost: Option<u64>,
    covered_provider_roles: &[ProviderRole],
    requires_tts: bool,
) -> Option<u64> {
    let cost = cost?;
    if evidence.measured_duration_millis == 0
        || !coverage_complete(covered_provider_roles, requires_tts)
    {
        return None;
    }
    let numerator = u128::from(cost).checked_mul(60_000)?;
    let value = numerator / u128::from(evidence.measured_duration_millis);
    u64::try_from(value).ok()
}

fn signal_complete(
    cost: Option<u64>,
    covered_provider_roles: &[ProviderRole],
    requires_tts: bool,
) -> bool {
    cost.is_some() && coverage_complete(covered_provider_roles, requires_tts)
}

fn coverage_complete(covered_provider_roles: &[ProviderRole], requires_tts: bool) -> bool {
    let covered: HashSet<_> = covered_provider_roles.iter().copied().collect();
    covered.len() == if requires_tts { 4 } else { 3 }
        && covered.contains(&ProviderRole::Stt)
        && covered.contains(&ProviderRole::Llm)
        && covered.contains(&ProviderRole::Avatar)
        && (!requires_tts || covered.contains(&ProviderRole::Tts))
}
