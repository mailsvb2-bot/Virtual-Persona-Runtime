use std::collections::HashSet;

use serde::Deserialize;

use crate::binding::{valid_git_sha, valid_sha256};
use crate::{
    BoundLabSessionEvidenceAggregate, LabSessionEvidenceAggregate, LatencyDistributionMillis,
    QualityEvidence,
    RT0_OWNER_LAB_SESSION_AGGREGATE_SCHEMA, RT0_OWNER_LAB_SESSION_BINDING_SCHEMA,
    RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA, Rt0ExitEvidence, Rt0ExitEvidenceError,
    Rt0ExitVerificationContext, bind_owner_lab_session_evidence, sha256_hex,
};

const RT0_LIVE_CONVERSATION_ATTEMPT_SCHEMA: &str = "rt0-live-conversation-attempt-0.1";

#[derive(Deserialize)]
struct ConversationAttemptBinding {
    schema_version: String,
    candidate_sha: String,
    provider_state_sha256: String,
}

pub(crate) fn validate_runtime_evidence(
    evidence: &Rt0ExitEvidence,
    context: Rt0ExitVerificationContext<'_>,
    provider_state_digest: &str,
) -> Result<(), Rt0ExitEvidenceError> {
    if evidence.conversation_attempt_sha256 != sha256_hex(context.conversation_attempt_bytes)
        || evidence.bound_session_aggregate_sha256
            != sha256_hex(context.bound_session_aggregate_bytes)
    {
        return Err(Rt0ExitEvidenceError::RuntimeEvidenceDigestMismatch);
    }

    let conversation: ConversationAttemptBinding =
        serde_json::from_slice(context.conversation_attempt_bytes)
            .map_err(|_| Rt0ExitEvidenceError::RuntimeEvidenceInvalid)?;
    if conversation.schema_version != RT0_LIVE_CONVERSATION_ATTEMPT_SCHEMA
        || !valid_git_sha(&conversation.candidate_sha)
        || !valid_sha256(&conversation.provider_state_sha256)
    {
        return Err(Rt0ExitEvidenceError::RuntimeEvidenceInvalid);
    }
    if conversation.candidate_sha != context.exact_candidate_sha {
        return Err(Rt0ExitEvidenceError::RuntimeEvidenceCandidateMismatch);
    }
    if conversation.provider_state_sha256 != provider_state_digest {
        return Err(Rt0ExitEvidenceError::RuntimeEvidenceProviderStateMismatch);
    }

    validate_bound_session_aggregate(
        context.bound_session_aggregate,
        context.exact_candidate_sha,
        provider_state_digest,
    )?;

    let recomputed = bind_owner_lab_session_evidence(
        context.session_snapshot_artifacts,
        context.provider_state_bytes,
        context.exact_candidate_sha,
    )
    .map_err(|_| Rt0ExitEvidenceError::RuntimeEvidenceInvalid)?;
    if recomputed != *context.bound_session_aggregate {
        return Err(Rt0ExitEvidenceError::RuntimeEvidenceInvalid);
    }
    validate_av_sync_quality_binding(evidence, &recomputed.aggregate)
}

fn validate_av_sync_quality_binding(
    evidence: &Rt0ExitEvidence,
    aggregate: &LabSessionEvidenceAggregate,
) -> Result<(), Rt0ExitEvidenceError> {
    let Some(av_sync) = aggregate.av_sync_absolute_offset else {
        return Err(Rt0ExitEvidenceError::RuntimeEvidenceInvalid);
    };
    if !aggregate.av_sync_proven || av_sync != evidence.quality.av_sync_absolute_offset {
        return Err(Rt0ExitEvidenceError::RuntimeEvidenceInvalid);
    }
    Ok(())
}

fn validate_bound_session_aggregate(
    bound: &BoundLabSessionEvidenceAggregate,
    exact_candidate_sha: &str,
    provider_state_digest: &str,
) -> Result<(), Rt0ExitEvidenceError> {
    if bound.schema_version != RT0_OWNER_LAB_SESSION_BINDING_SCHEMA
        || !valid_git_sha(&bound.candidate_sha)
        || !valid_sha256(&bound.provider_state_sha256)
        || bound.snapshot_sha256.is_empty()
        || bound.aggregate.schema_version != RT0_OWNER_LAB_SESSION_AGGREGATE_SCHEMA
        || bound.aggregate.source_schema_version != RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA
        || bound.aggregate.sessions == 0
        || usize::try_from(bound.aggregate.sessions).ok() != Some(bound.snapshot_sha256.len())
    {
        return Err(Rt0ExitEvidenceError::RuntimeEvidenceInvalid);
    }
    if bound.candidate_sha != exact_candidate_sha {
        return Err(Rt0ExitEvidenceError::RuntimeEvidenceCandidateMismatch);
    }
    if bound.provider_state_sha256 != provider_state_digest {
        return Err(Rt0ExitEvidenceError::RuntimeEvidenceProviderStateMismatch);
    }

    let mut unique = HashSet::new();
    if bound
        .snapshot_sha256
        .iter()
        .any(|digest| !valid_sha256(digest) || !unique.insert(digest))
    {
        return Err(Rt0ExitEvidenceError::RuntimeEvidenceInvalid);
    }
    for distribution in [
        bound.aggregate.stt_latency,
        bound.aggregate.llm_latency,
        bound.aggregate.avatar_submit_latency,
        bound.aggregate.server_total_latency,
        bound.aggregate.av_sync_absolute_offset,
        bound.aggregate.first_meaningful_audio,
        bound.aggregate.interruption_stop,
        bound.aggregate.first_useful_video,
        bound.aggregate.recoverable_reconnect,
    ]
    .into_iter()
    .flatten()
    {
        validate_distribution(distribution)?;
    }
    Ok(())
}

pub(crate) fn validate_quality_latencies(
    quality: &QualityEvidence,
) -> Result<(), Rt0ExitEvidenceError> {
    for distribution in [
        quality.text_first_meaningful_response,
        quality.first_meaningful_audio,
        quality.interruption_stop,
        quality.first_useful_video,
        quality.av_sync_absolute_offset,
        quality.recoverable_reconnect,
    ] {
        validate_distribution(distribution)?;
    }
    Ok(())
}

fn validate_distribution(
    distribution: LatencyDistributionMillis,
) -> Result<(), Rt0ExitEvidenceError> {
    if distribution.samples == 0 || distribution.p50 > distribution.p95 {
        return Err(Rt0ExitEvidenceError::InvalidLatencyDistribution);
    }
    Ok(())
}
