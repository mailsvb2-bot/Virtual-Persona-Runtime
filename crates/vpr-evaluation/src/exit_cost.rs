use crate::{CostEvidence, EvidenceOrigin, Rt0ExitFailureCode};

pub(crate) fn evaluate_cost(evidence: &CostEvidence, failures: &mut Vec<Rt0ExitFailureCode>) {
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

pub(crate) fn cost_per_minute(cost: Option<u64>, measured_duration_millis: u64) -> Option<u64> {
    let cost = cost?;
    if measured_duration_millis == 0 {
        return None;
    }
    let numerator = u128::from(cost).checked_mul(60_000)?;
    let value = numerator / u128::from(measured_duration_millis);
    u64::try_from(value).ok()
}
