use crate::{
    AcceptanceEvidence, CheckStatus, ConversationEvidence, ConversationPairEvidence,
    EvidenceOrigin, HumanEvaluationEvidence, PrivacyPermissionEvidence, QualityEvidence,
    RecordStatus, Rt0ExitFailureCode,
};

pub(crate) fn evaluate_conversations(
    evidence: &ConversationPairEvidence,
    failures: &mut Vec<Rt0ExitFailureCode>,
) {
    if evidence.owner.origin != EvidenceOrigin::Real {
        failures.push(Rt0ExitFailureCode::OwnerConversationNotReal);
    }
    if !conversation_complete(&evidence.owner) {
        failures.push(Rt0ExitFailureCode::OwnerConversationIncomplete);
    }
    if evidence.owner.interruption_exercised != CheckStatus::Passed {
        failures.push(Rt0ExitFailureCode::OwnerInterruptionNotExercised);
    }
    if evidence.visitor.origin != EvidenceOrigin::Real {
        failures.push(Rt0ExitFailureCode::VisitorConversationNotReal);
    }
    if !conversation_complete(&evidence.visitor) {
        failures.push(Rt0ExitFailureCode::VisitorConversationIncomplete);
    }
}

fn conversation_complete(evidence: &ConversationEvidence) -> bool {
    evidence.russian == CheckStatus::Passed
        && evidence.voice == CheckStatus::Passed
        && evidence.video == CheckStatus::Passed
        && evidence.completed_turns > 0
}

pub(crate) fn evaluate_acceptance(
    evidence: &AcceptanceEvidence,
    failures: &mut Vec<Rt0ExitFailureCode>,
) {
    if evidence.origin != EvidenceOrigin::Real {
        failures.push(Rt0ExitFailureCode::AcceptanceEvidenceNotReal);
    }
    if evidence.owner_happy_path != CheckStatus::Passed
        || evidence.visitor_happy_path != CheckStatus::Passed
        || evidence.correction_path != CheckStatus::Passed
        || evidence.failure_recovery_path != CheckStatus::Passed
        || evidence.revoke_deny_path != CheckStatus::Passed
    {
        failures.push(Rt0ExitFailureCode::AcceptanceMatrixIncomplete);
    }
}

pub(crate) fn evaluate_quality(evidence: &QualityEvidence, failures: &mut Vec<Rt0ExitFailureCode>) {
    if evidence.origin != EvidenceOrigin::Real {
        failures.push(Rt0ExitFailureCode::QualityEvidenceNotReal);
    }
    if evidence.text_first_meaningful_response.p50 > 1_000
        || evidence.text_first_meaningful_response.p95 > 2_500
    {
        failures.push(Rt0ExitFailureCode::TextLatencyExceeded);
    }
    if evidence.first_meaningful_audio.p50 > 1_500 || evidence.first_meaningful_audio.p95 > 3_000 {
        failures.push(Rt0ExitFailureCode::AudioLatencyExceeded);
    }
    if evidence.interruption_stop.p95 > 500 {
        failures.push(Rt0ExitFailureCode::InterruptionLatencyExceeded);
    }
    if evidence.first_useful_video.p95 > 2_500 {
        failures.push(Rt0ExitFailureCode::VideoLatencyExceeded);
    }
    if evidence.av_sync_absolute_offset.p95 > 120 {
        failures.push(Rt0ExitFailureCode::AvSyncExceeded);
    }
    if evidence.recoverable_reconnect.p95 > 5_000 {
        failures.push(Rt0ExitFailureCode::ReconnectLatencyExceeded);
    }
}

pub(crate) fn evaluate_privacy(
    evidence: &PrivacyPermissionEvidence,
    failures: &mut Vec<Rt0ExitFailureCode>,
) {
    if evidence.origin != EvidenceOrigin::Real {
        failures.push(Rt0ExitFailureCode::PrivacyEvidenceNotReal);
    }
    if evidence.permission_suite != CheckStatus::Passed {
        failures.push(Rt0ExitFailureCode::PermissionSuiteNotPassed);
    }
    if evidence.accepted_private_context_leakage != 0 {
        failures.push(Rt0ExitFailureCode::PrivateContextLeakageAccepted);
    }
    if evidence.accepted_false_owner_attribution != 0 {
        failures.push(Rt0ExitFailureCode::FalseOwnerAttributionAccepted);
    }
    if evidence.revocation != CheckStatus::Passed {
        failures.push(Rt0ExitFailureCode::RevocationNotVerified);
    }
    if evidence.egress_denial != CheckStatus::Passed {
        failures.push(Rt0ExitFailureCode::EgressDenialNotVerified);
    }
}

pub(crate) fn evaluate_human(
    evidence: &HumanEvaluationEvidence,
    failures: &mut Vec<Rt0ExitFailureCode>,
) {
    if evidence.origin != EvidenceOrigin::Real {
        failures.push(Rt0ExitFailureCode::HumanEvaluationNotReal);
    }
    if evidence.owner_human_participant_verified != CheckStatus::Passed {
        failures.push(Rt0ExitFailureCode::OwnerHumanParticipantNotVerified);
    }
    if evidence.visitor_distinct_non_owner_human_verified != CheckStatus::Passed {
        failures.push(Rt0ExitFailureCode::VisitorDistinctNonOwnerHumanNotVerified);
    }
    if evidence.reviewer_count == 0
        || evidence.dimensions.voice_similarity != RecordStatus::Recorded
        || evidence.dimensions.voice_naturalness != RecordStatus::Recorded
        || evidence.dimensions.appearance_plausibility != RecordStatus::Recorded
        || evidence.dimensions.persona_similarity != RecordStatus::Recorded
        || evidence.dimensions.conversation_naturalness != RecordStatus::Recorded
    {
        failures.push(Rt0ExitFailureCode::HumanEvaluationIncomplete);
    }
    if evidence.usable_for_continuation != CheckStatus::Passed {
        failures.push(Rt0ExitFailureCode::HumanEvaluationNotUsable);
    }
}
