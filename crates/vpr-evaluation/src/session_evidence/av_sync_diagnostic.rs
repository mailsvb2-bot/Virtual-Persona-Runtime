use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use super::{LabSessionAggregateError, LabSessionEvidenceSnapshot, LabVoiceAttemptStatus};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LabAvSyncTrackIssue {
    StatsUnavailable,
    TimestampUnavailable,
    SenderReportTimingUnavailable,
    AmbiguousStreams,
    NoUniqueActiveStream,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LabAvSyncDiagnosticInput {
    pub session_sequence: u64,
    pub request_sequence: u64,
    pub attempts: u32,
    pub audio_issue: Option<LabAvSyncTrackIssue>,
    pub video_issue: Option<LabAvSyncTrackIssue>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LabAvSyncDiagnostic {
    pub request_sequence: u64,
    pub attempts: u32,
    pub audio_issue: Option<LabAvSyncTrackIssue>,
    pub video_issue: Option<LabAvSyncTrackIssue>,
}

pub(super) fn validate_av_sync_diagnostics(
    snapshot: &LabSessionEvidenceSnapshot,
    request_status: &BTreeMap<u64, LabVoiceAttemptStatus>,
    audio_requests: &HashSet<u64>,
    proven_requests: &HashSet<u64>,
) -> Result<(), LabSessionAggregateError> {
    let mut unique = HashSet::new();
    for diagnostic in &snapshot.av_sync_diagnostics {
        if diagnostic.request_sequence == 0
            || diagnostic.attempts == 0
            || diagnostic.audio_issue.is_none() && diagnostic.video_issue.is_none()
            || request_status.get(&diagnostic.request_sequence)
                != Some(&LabVoiceAttemptStatus::Completed)
            || !audio_requests.contains(&diagnostic.request_sequence)
            || proven_requests.contains(&diagnostic.request_sequence)
            || !unique.insert(diagnostic.request_sequence)
        {
            return Err(LabSessionAggregateError::InvalidMediaEvidence);
        }
    }
    Ok(())
}
