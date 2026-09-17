use serde::{Deserialize, Serialize};

use crate::binding::valid_sha256;

pub const RT0_LIVE_PROVIDER_PROBE_SCHEMA: &str = "rt0-live-provider-probe-0.2";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProbeUsage {
    pub input_units: Option<u64>,
    pub input_unit: Option<String>,
    pub output_units: Option<u64>,
    pub output_unit: Option<String>,
    pub estimated_cost_microunits: Option<u64>,
    pub provider_charge_microunits: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SttProbeEvidence {
    pub latency_millis: u64,
    pub transcript_chars: u64,
    pub usage: ProbeUsage,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LlmProbeEvidence {
    pub latency_millis: u64,
    pub output_chars: u64,
    pub usage: ProbeUsage,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TtsProbeEvidence {
    pub latency_millis: u64,
    pub audio_sha256: String,
    pub audio_millis: u64,
    pub usage: ProbeUsage,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AvatarProbeEvidence {
    pub open_millis: u64,
    pub close_millis: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LiveProviderProbeReceipt {
    pub schema_version: String,
    pub candidate_sha: String,
    pub provider_state_sha256: String,
    pub input_audio_sha256: String,
    pub input_audio_millis: u64,
    pub scope: String,
    pub conversation_evidence: bool,
    pub output_delivery_proven: bool,
    pub stt: SttProbeEvidence,
    pub llm: LlmProbeEvidence,
    pub tts: TtsProbeEvidence,
    pub avatar: AvatarProbeEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiveProviderProbeValidationError {
    Invalid,
    CandidateMismatch,
    ProviderStateMismatch,
}

pub(crate) fn validate_live_provider_probe(
    probe: &LiveProviderProbeReceipt,
    exact_candidate_sha: &str,
    provider_state_sha256: &str,
) -> Result<(), LiveProviderProbeValidationError> {
    if probe.schema_version != RT0_LIVE_PROVIDER_PROBE_SCHEMA
        || probe.scope != "credentialed_provider_reachability_only"
        || probe.conversation_evidence
        || probe.output_delivery_proven
        || !valid_sha256(&probe.input_audio_sha256)
        || probe.input_audio_millis == 0
        || probe.input_audio_millis > 30_000
        || probe.stt.transcript_chars == 0
        || probe.llm.output_chars == 0
        || !valid_sha256(&probe.tts.audio_sha256)
        || probe.tts.audio_millis == 0
        || probe.tts.audio_millis > 30_000
    {
        return Err(LiveProviderProbeValidationError::Invalid);
    }
    if probe.candidate_sha != exact_candidate_sha {
        return Err(LiveProviderProbeValidationError::CandidateMismatch);
    }
    if probe.provider_state_sha256 != provider_state_sha256 {
        return Err(LiveProviderProbeValidationError::ProviderStateMismatch);
    }
    Ok(())
}
