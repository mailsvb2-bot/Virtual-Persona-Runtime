use std::collections::HashSet;

use serde::Deserialize;

use crate::binding::{valid_git_sha, valid_sha256};
use crate::{
    BoundLabSessionEvidenceAggregate, CheckStatus, ConversationEvidence, LabMediaEvidenceKind,
    LabSessionEvidenceAggregate, LabSessionEvidenceSnapshot, LabVoiceAttemptStatus,
    LatencyDistributionMillis, ParticipantRole, QualityEvidence,
    RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE, RT0_OWNER_LAB_SESSION_AGGREGATE_SCHEMA,
    RT0_OWNER_LAB_SESSION_BINDING_SCHEMA, RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA, Rt0ExitEvidence,
    Rt0ExitEvidenceError, Rt0ExitVerificationContext, bind_owner_lab_session_evidence, sha256_hex,
};

const RT0_LIVE_CONVERSATION_ATTEMPT_SCHEMA: &str = "rt0-live-conversation-attempt-0.1";

#[derive(Deserialize)]
struct ConversationTurnBinding {
    audience: ParticipantRole,
    input_audio_sha256: String,
    transcript_sha256: String,
    transcript_chars: u64,
    reply_sha256: String,
    reply_chars: u64,
    locale: String,
}

#[derive(Deserialize)]
struct ConversationAttemptBinding {
    schema_version: String,
    candidate_sha: String,
    provider_state_sha256: String,
    profile_input_sha256: String,
    persona_id_sha256: String,
    persona_version: u64,
    reviewed_claims: usize,
    owner: ConversationTurnBinding,
    visitor: ConversationTurnBinding,
    conversation_attempted: bool,
    provider_output_submitted: bool,
}

#[derive(Default)]
struct RoleConversationProof {
    completed_turns: u32,
    sessions_with_completed_turns: u32,
    playback_sessions: u32,
    video_sessions: u32,
    interruption_exercised: bool,
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

    validate_rt0_conversation_evidence_binding(
        evidence,
        context.conversation_attempt_bytes,
        context.session_snapshot_artifacts,
        context.exact_candidate_sha,
        provider_state_digest,
    )?;

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

/// Validates that real-conversation claims are derived from the credentialed conversation receipt
/// plus exact raw role-bound Owner Lab session snapshots.
///
/// # Errors
/// Returns a runtime-evidence error for malformed, stale, cross-candidate/provider, role-mismatched,
/// or detached conversation claims.
pub fn validate_rt0_conversation_evidence_binding(
    evidence: &Rt0ExitEvidence,
    conversation_attempt_bytes: &[u8],
    session_snapshot_artifacts: &[&[u8]],
    exact_candidate_sha: &str,
    provider_state_digest: &str,
) -> Result<(), Rt0ExitEvidenceError> {
    let conversation: ConversationAttemptBinding = serde_json::from_slice(conversation_attempt_bytes)
        .map_err(|_| Rt0ExitEvidenceError::RuntimeEvidenceInvalid)?;
    validate_conversation_attempt_binding(
        &conversation,
        exact_candidate_sha,
        provider_state_digest,
    )?;
    let snapshots = parse_role_bound_snapshots(session_snapshot_artifacts)?;
    let owner = derive_role_conversation_proof(&snapshots, ParticipantRole::Owner)?;
    let visitor = derive_role_conversation_proof(&snapshots, ParticipantRole::Visitor)?;
    if !conversation_claim_matches(
        &evidence.conversations.owner,
        &conversation.owner,
        ParticipantRole::Owner,
        &owner,
    ) || !conversation_claim_matches(
        &evidence.conversations.visitor,
        &conversation.visitor,
        ParticipantRole::Visitor,
        &visitor,
    ) {
        return Err(Rt0ExitEvidenceError::RuntimeEvidenceInvalid);
    }
    Ok(())
}

fn validate_conversation_attempt_binding(
    conversation: &ConversationAttemptBinding,
    exact_candidate_sha: &str,
    provider_state_digest: &str,
) -> Result<(), Rt0ExitEvidenceError> {
    if conversation.schema_version != RT0_LIVE_CONVERSATION_ATTEMPT_SCHEMA
        || !valid_git_sha(&conversation.candidate_sha)
        || !valid_sha256(&conversation.provider_state_sha256)
        || !valid_sha256(&conversation.profile_input_sha256)
        || !valid_sha256(&conversation.persona_id_sha256)
        || conversation.persona_version == 0
        || conversation.reviewed_claims == 0
        || !conversation.conversation_attempted
        || !conversation.provider_output_submitted
        || !valid_conversation_turn(&conversation.owner, ParticipantRole::Owner)
        || !valid_conversation_turn(&conversation.visitor, ParticipantRole::Visitor)
    {
        return Err(Rt0ExitEvidenceError::RuntimeEvidenceInvalid);
    }
    if conversation.candidate_sha != exact_candidate_sha {
        return Err(Rt0ExitEvidenceError::RuntimeEvidenceCandidateMismatch);
    }
    if conversation.provider_state_sha256 != provider_state_digest {
        return Err(Rt0ExitEvidenceError::RuntimeEvidenceProviderStateMismatch);
    }
    Ok(())
}

fn valid_conversation_turn(turn: &ConversationTurnBinding, role: ParticipantRole) -> bool {
    turn.audience == role
        && valid_sha256(&turn.input_audio_sha256)
        && valid_sha256(&turn.transcript_sha256)
        && turn.transcript_chars > 0
        && valid_sha256(&turn.reply_sha256)
        && turn.reply_chars > 0
        && !turn.locale.trim().is_empty()
}

fn parse_role_bound_snapshots(
    artifacts: &[&[u8]],
) -> Result<Vec<LabSessionEvidenceSnapshot>, Rt0ExitEvidenceError> {
    if artifacts.is_empty() {
        return Err(Rt0ExitEvidenceError::RuntimeEvidenceInvalid);
    }
    artifacts
        .iter()
        .map(|bytes| {
            serde_json::from_slice::<LabSessionEvidenceSnapshot>(bytes)
                .map_err(|_| Rt0ExitEvidenceError::RuntimeEvidenceInvalid)
        })
        .collect()
}

fn derive_role_conversation_proof(
    snapshots: &[LabSessionEvidenceSnapshot],
    role: ParticipantRole,
) -> Result<RoleConversationProof, Rt0ExitEvidenceError> {
    let mut proof = RoleConversationProof::default();
    for snapshot in snapshots.iter().filter(|snapshot| snapshot.participant_role == role) {
        if snapshot.schema_version != RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA
            || snapshot.scope != RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE
            || snapshot.session_sequence == 0
        {
            return Err(Rt0ExitEvidenceError::RuntimeEvidenceInvalid);
        }
        let completed = u32::try_from(
            snapshot
                .voice_attempts
                .iter()
                .filter(|attempt| attempt.status == LabVoiceAttemptStatus::Completed)
                .count(),
        )
        .map_err(|_| Rt0ExitEvidenceError::RuntimeEvidenceInvalid)?;
        if completed == 0 {
            continue;
        }
        proof.completed_turns = proof
            .completed_turns
            .checked_add(completed)
            .ok_or(Rt0ExitEvidenceError::RuntimeEvidenceInvalid)?;
        proof.sessions_with_completed_turns = proof
            .sessions_with_completed_turns
            .checked_add(1)
            .ok_or(Rt0ExitEvidenceError::RuntimeEvidenceInvalid)?;
        if snapshot.canonical_playback_proven {
            proof.playback_sessions = proof
                .playback_sessions
                .checked_add(1)
                .ok_or(Rt0ExitEvidenceError::RuntimeEvidenceInvalid)?;
        }
        if snapshot
            .media_events
            .iter()
            .any(|event| event.kind == LabMediaEvidenceKind::VideoReady)
        {
            proof.video_sessions = proof
                .video_sessions
                .checked_add(1)
                .ok_or(Rt0ExitEvidenceError::RuntimeEvidenceInvalid)?;
        }
        proof.interruption_exercised |= snapshot
            .media_events
            .iter()
            .any(|event| event.kind == LabMediaEvidenceKind::InterruptionStopped);
    }
    Ok(proof)
}

fn conversation_claim_matches(
    claim: &ConversationEvidence,
    turn: &ConversationTurnBinding,
    role: ParticipantRole,
    proof: &RoleConversationProof,
) -> bool {
    let voice_proven = proof.completed_turns > 0
        && proof.playback_sessions == proof.sessions_with_completed_turns;
    let video_proven = proof.completed_turns > 0
        && proof.video_sessions == proof.sessions_with_completed_turns;
    claim.role == role
        && claim.completed_turns == proof.completed_turns
        && claim.russian == check_status(is_russian_locale(&turn.locale))
        && claim.voice == check_status(voice_proven)
        && claim.video == check_status(video_proven)
        && claim.interruption_exercised == check_status(proof.interruption_exercised)
}

fn check_status(passed: bool) -> CheckStatus {
    if passed {
        CheckStatus::Passed
    } else {
        CheckStatus::Failed
    }
}

fn is_russian_locale(locale: &str) -> bool {
    let normalized = locale.trim().to_ascii_lowercase();
    normalized == "ru" || normalized.starts_with("ru-") || normalized.starts_with("ru_")
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
