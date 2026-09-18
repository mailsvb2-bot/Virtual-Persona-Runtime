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
    pub provider: String,
    pub model_or_representation: String,
    pub configuration_fingerprint_sha256: String,
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
pub enum LiveProviderProbeValidationError {
    Invalid,
    CandidateMismatch,
    ProviderStateMismatch,
}

/// Validates one sanitized credentialed live-provider probe against its exact candidate and
/// provider-state binding.
///
/// # Errors
/// Returns a stable validation error when the receipt is structurally invalid, claims unsupported
/// conversation/output proof, carries malformed or empty evidence, or is stale for the exact
/// candidate/provider state.
pub fn validate_live_provider_probe(
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
        || probe.tts.provider.trim().is_empty()
        || probe.tts.model_or_representation.trim().is_empty()
        || !valid_sha256(&probe.tts.configuration_fingerprint_sha256)
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
