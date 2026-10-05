mod conversation;
mod probe;

use serde::Serialize;
use vpr_evaluation::{ProviderStateManifest, sha256_hex};
use vpr_owner_lab::ProviderBundle;

pub use conversation::{
    LiveConversationAttemptError, LiveConversationAttemptReceipt, LiveConversationAttemptRun,
    LiveConversationClaimInput, LiveConversationProfileInput, LiveConversationTurnReceipt,
    ProofStatus, RT0_LIVE_CONVERSATION_ATTEMPT_SCHEMA, RT0_LIVE_CONVERSATION_PROFILE_SCHEMA,
    run_live_conversation_attempt, run_live_conversation_attempt_with_avatar_probe,
    validate_live_conversation_inputs,
};
pub use probe::{
    LiveProviderProbeCoreEvidence, LiveProviderProbeError, run_provider_probe,
    run_provider_probe_core, validate_provider_probe_audio,
};
pub use vpr_evaluation::{
    AvatarProbeEvidence, LiveProviderProbeReceipt, LlmProbeEvidence, ProbeUsage,
    RT0_LIVE_PROVIDER_PROBE_SCHEMA, SttProbeEvidence,
};

pub const RT0_LIVE_PROOF_PREFLIGHT_SCHEMA: &str = "rt0-live-proof-preflight-0.1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConfigurationInspection {
    pub candidate_sha: String,
    pub provider_state_sha256: String,
    pub provider_state: ProviderStateManifest,
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
    prepare_provider_configuration(candidate_sha, worktree_clean, false)
}

/// Builds the live-proof provider bundle from environment variables only.
///
/// This bypasses any ambient Windows Credential Manager profile so hermetic callers can prove that
/// incomplete explicit configuration fails closed.
///
/// # Errors
/// Returns the same fail-closed preflight errors as [`prepare`].
pub fn prepare_environment_only(
    candidate_sha: &str,
    worktree_clean: bool,
    egress_authorized: bool,
) -> Result<PreparedLiveProof, LiveProofPreflightError> {
    if !egress_authorized {
        return Err(LiveProofPreflightError::EgressNotAuthorized);
    }
    prepare_provider_configuration(candidate_sha, worktree_clean, true)
}

fn prepare_provider_configuration(
    candidate_sha: &str,
    worktree_clean: bool,
    environment_only: bool,
) -> Result<PreparedLiveProof, LiveProofPreflightError> {
    if !valid_git_sha(candidate_sha) {
        return Err(LiveProofPreflightError::CandidateInvalid);
    }
    if !worktree_clean {
        return Err(LiveProofPreflightError::WorktreeDirty);
    }
    let providers = if environment_only {
        ProviderBundle::from_environment(true)
    } else {
        ProviderBundle::from_env(true)
    }
    .map_err(|_| LiveProofPreflightError::ProviderConfigurationInvalid)?;
    let provider_state = providers
        .provider_state_manifest()
        .map_err(|_| LiveProofPreflightError::ProviderConfigurationInvalid)?;
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

/// Inspects the exact local RT0 candidate and configured provider composition without egress.
///
/// Input-file validation is deliberately outside this API; callers must not infer that private
/// evidence inputs were checked merely because provider configuration is valid.
///
/// # Errors
/// Fails closed for an invalid/dirty candidate or incomplete/rejected provider configuration.
pub fn inspect_provider_configuration(
    candidate_sha: &str,
    worktree_clean: bool,
) -> Result<ProviderConfigurationInspection, LiveProofPreflightError> {
    let prepared = prepare_provider_configuration(candidate_sha, worktree_clean, false)?;
    let receipt = prepared.receipt();
    Ok(ProviderConfigurationInspection {
        candidate_sha: receipt.candidate_sha.clone(),
        provider_state_sha256: receipt.provider_state_sha256.clone(),
        provider_state: receipt.provider_state.clone(),
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

/// Environment-only variant of [`preflight`] for hermetic callers.
///
/// # Errors
/// Returns the same fail-closed errors as [`preflight`].
pub fn preflight_environment_only(
    candidate_sha: &str,
    worktree_clean: bool,
    egress_authorized: bool,
) -> Result<LiveProofPreflightReceipt, LiveProofPreflightError> {
    prepare_environment_only(candidate_sha, worktree_clean, egress_authorized)
        .map(|prepared| prepared.receipt)
}

/// Environment-only variant of [`inspect_provider_configuration`].
///
/// # Errors
/// Fails closed for an invalid/dirty candidate or incomplete environment-only provider configuration.
pub fn inspect_provider_configuration_environment_only(
    candidate_sha: &str,
    worktree_clean: bool,
) -> Result<ProviderConfigurationInspection, LiveProofPreflightError> {
    let prepared = prepare_provider_configuration(candidate_sha, worktree_clean, true)?;
    let receipt = prepared.receipt();
    Ok(ProviderConfigurationInspection {
        candidate_sha: receipt.candidate_sha.clone(),
        provider_state_sha256: receipt.provider_state_sha256.clone(),
        provider_state: receipt.provider_state.clone(),
    })
}

fn valid_git_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
