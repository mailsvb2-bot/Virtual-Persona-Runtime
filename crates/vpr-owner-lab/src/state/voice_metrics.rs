use std::time::Instant;

use vpr_integration::UsageEvidence;

use super::voice::LabVoiceUsage;

pub(super) fn map_usage(usage: &UsageEvidence) -> LabVoiceUsage {
    LabVoiceUsage {
        input_units: usage.input_units,
        output_units: usage.output_units,
        estimated_cost_microunits: usage.estimated_cost_microunits,
        provider_charge_microunits: usage.provider_charge_microunits,
    }
}

pub(super) fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}
