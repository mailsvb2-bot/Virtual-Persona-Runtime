use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SessionUsageEvidence {
    pub input_units: Option<u64>,
    pub output_units: Option<u64>,
    pub estimated_cost_microunits: Option<u64>,
    pub provider_charge_microunits: Option<u64>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LabTextAttemptStatus {
    Pending,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LabTextAttemptEvidence {
    pub request_sequence: u64,
    pub canonical_turn_sequence: Option<u64>,
    pub canonical_output_sequence: Option<u64>,
    pub status: LabTextAttemptStatus,
    pub failure_code: Option<String>,
    pub first_meaningful_response_millis: Option<u64>,
    pub server_total_millis: Option<u64>,
    pub llm_usage: Option<SessionUsageEvidence>,
}
