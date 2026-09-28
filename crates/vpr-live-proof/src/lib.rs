mod conversation;
mod probe;

use serde::Serialize;
use vpr_evaluation::{
    ProviderRole, ProviderStateBinding, ProviderStateManifest, RT0_PROVIDER_STATE_SCHEMA,
    sha256_hex,
};
use vpr_owner_lab::{ProviderBundle, ProviderDescriptor};

pub use conversation::{
    LiveConversationAttemptError, LiveConversationAttemptReceipt, LiveConversationClaimInput,
    LiveConversationProfileInput, LiveConversationTurnReceipt, ProofStatus,
    RT0_LIVE_CONVERSATION_ATTEMPT_SCHEMA, RT0_LIVE_CONVERSATION_PROFILE_SCHEMA,
    run_live_conversation_attempt, validate_live_conversation_inputs,
};
pub use probe::{LiveProviderProbeError, run_provider_probe, validate_provider_probe_audio};
pub use vpr_evaluation::{
    AvatarProbeEvidence, LiveProviderProbeReceipt, LlmProbeEvidence, ProbeUsage,
    RT0_LIVE_PROVIDER_PROBE_SCHEMA, SttProbeEvidence,
};

pub const RT0_LIVE_PROOF_PREFLIGHT_SCHEMA: &str = "rt0-live-proof-preflight-0.1";
pub const RT0_LIVE_PROOF_DOCTOR_SCHEMA: &str = "rt0-live-proof-doctor-0.1";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LiveProofDoctorReceipt {
    pub schema_version: String,
    pub candidate_sha: String,
    pub provider_state_sha256: String,
    pub provider_state: ProviderStateManifest,
    pub input_validation_passed: bool,
    pub provider_configuration_passed: bool,
    pub egress_performed: bool,
    pub release_evidence: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LiveProofPreflightReceipt {
    pub schema_version: String,
    pub candidate_sha: String,
    pub provider_state_sha256: String,
    pub provider_state: ProviderStateManifest,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LiveProofPreflightError {
    EgressNotAuthorized,
    CandidateInvalid,
    CandidateChanged,
    WorktreeDirty,
    ProviderConfigurationInvalid,
    ProviderStateSerializationFailed,
    OutputPathInvalid,
    OutputPathInsideWorktree,
}

/// Shared provider composition and sanitized binding prepared for one exact live-proof candidate.
pub struct PreparedLiveProof {
    receipt: LiveProofPreflightReceipt,
    providers: ProviderBundle,
}

impl PreparedLiveProof {
    #[must_use]
    pub const fn receipt(&self) -> &LiveProofPreflightReceipt {
        &self.receipt
    }

    pub(crate) fn into_parts(self) -> (LiveProofPreflightReceipt, ProviderBundle) {
        (self.receipt, self.providers)
    }
}

/// Builds the shared credentialed provider bundle and its sanitized exact-candidate receipt.
///
/// # Errors
/// Returns the same fail-closed preflight errors as [`preflight`].
pub fn prepare(
    candidate_sha: &str,
    worktree_clean: bool,
    egress_authorized: bool,
) -> Result<PreparedLiveProof, LiveProofPreflightError> {
    if !egress_authorized {
        return Err(LiveProofPreflightError::EgressNotAuthorized);
    }
    prepare_provider_configuration(candidate_sha, worktree_clean)
}

fn prepare_provider_configuration(
    candidate_sha: &str,
    worktree_clean: bool,
) -> Result<PreparedLiveProof, LiveProofPreflightError> {
    if !valid_git_sha(candidate_sha) {
        return Err(LiveProofPreflightError::CandidateInvalid);
    }
    if !worktree_clean {
        return Err(LiveProofPreflightError::WorktreeDirty);
    }
    let providers = ProviderBundle::from_env(true)
        .map_err(|_| LiveProofPreflightError::ProviderConfigurationInvalid)?;
    let provider_state = provider_state(&providers)?;
    let provider_state_bytes = serde_json::to_vec_pretty(&provider_state)
        .map_err(|_| LiveProofPreflightError::ProviderStateSerializationFailed)?;
    Ok(PreparedLiveProof {
        receipt: LiveProofPreflightReceipt {
            schema_version: RT0_LIVE_PROOF_PREFLIGHT_SCHEMA.into(),
            candidate_sha: candidate_sha.into(),
            provider_state_sha256: sha256_hex(&provider_state_bytes),
            provider_state,
        },
        providers,
    })
}

/// Validates the exact local RT0 candidate and configured provider composition without egress.
///
/// This is an operator readiness check only. It performs no provider calls, writes no evidence
/// artifacts, and cannot satisfy any RT0 exit criterion.
///
/// # Errors
/// Fails closed for an invalid/dirty candidate or incomplete/rejected provider configuration.
pub fn doctor(
    candidate_sha: &str,
    worktree_clean: bool,
) -> Result<LiveProofDoctorReceipt, LiveProofPreflightError> {
    let prepared = prepare_provider_configuration(candidate_sha, worktree_clean)?;
    let receipt = prepared.receipt();
    Ok(LiveProofDoctorReceipt {
        schema_version: RT0_LIVE_PROOF_DOCTOR_SCHEMA.into(),
        candidate_sha: receipt.candidate_sha.clone(),
        provider_state_sha256: receipt.provider_state_sha256.clone(),
        provider_state: receipt.provider_state.clone(),
        input_validation_passed: true,
        provider_configuration_passed: true,
        egress_performed: false,
        release_evidence: false,
    })
}

/// Creates a sanitized provider-state receipt after fail-closed live-proof preflight checks.
///
/// # Errors
/// Returns a stable error when egress is not explicitly authorized, the exact candidate is invalid,
/// the worktree is dirty, provider configuration is incomplete, or the sanitized state cannot be encoded.
pub fn preflight(
    candidate_sha: &str,
    worktree_clean: bool,
    egress_authorized: bool,
) -> Result<LiveProofPreflightReceipt, LiveProofPreflightError> {
    prepare(candidate_sha, worktree_clean, egress_authorized).map(|prepared| prepared.receipt)
}

fn provider_state(
    bundle: &ProviderBundle,
) -> Result<ProviderStateManifest, LiveProofPreflightError> {
    let stt = bundle
        .stt_descriptor
        .as_ref()
        .ok_or(LiveProofPreflightError::ProviderConfigurationInvalid)?;
    let llm = bundle
        .llm_descriptor
        .as_ref()
        .ok_or(LiveProofPreflightError::ProviderConfigurationInvalid)?;
    Ok(ProviderStateManifest {
        schema_version: RT0_PROVIDER_STATE_SCHEMA.into(),
        providers: vec![
            binding(ProviderRole::Stt, stt),
            binding(ProviderRole::Llm, llm),
            binding(ProviderRole::Avatar, &bundle.avatar_descriptor),
        ],
    })
}

fn binding(role: ProviderRole, descriptor: &ProviderDescriptor) -> ProviderStateBinding {
    ProviderStateBinding {
        role,
        provider: descriptor.provider.clone(),
        model_or_representation: descriptor.model_or_representation.clone(),
        configuration_fingerprint_sha256: descriptor.configuration_fingerprint_sha256.clone(),
    }
}

fn valid_git_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
