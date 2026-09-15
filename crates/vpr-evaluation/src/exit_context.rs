use crate::{
    BoundLabSessionEvidenceAggregate, GoldenEvidenceBundle, LiveProviderProbeReceipt,
    ProviderStateManifest,
};

#[derive(Debug, Clone, Copy)]
pub struct Rt0ExitVerificationContext<'a> {
    pub exit_evidence_bytes: &'a [u8],
    pub golden_report_bytes: &'a [u8],
    pub golden_evidence_bundle: &'a GoldenEvidenceBundle,
    pub golden_evidence_bytes: &'a [u8],
    pub provider_state: &'a ProviderStateManifest,
    pub provider_state_bytes: &'a [u8],
    pub live_provider_probe: &'a LiveProviderProbeReceipt,
    pub live_provider_probe_bytes: &'a [u8],
    pub conversation_attempt_bytes: &'a [u8],
    pub bound_session_aggregate: &'a BoundLabSessionEvidenceAggregate,
    pub bound_session_aggregate_bytes: &'a [u8],
    pub release_spec_bytes: &'a [u8],
    pub exact_candidate_sha: &'a str,
}
