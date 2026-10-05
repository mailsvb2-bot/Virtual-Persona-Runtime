use std::collections::HashSet;

use super::{
    LabMediaEvidenceKind, LabSessionAggregateError, LabSessionEvidenceSnapshot,
};

pub(super) struct PlaybackAccounting {
    pub(super) playback_requests: HashSet<u64>,
    pub(super) audio_requests: HashSet<u64>,
    pub(super) derived_playback: bool,
}

pub(super) fn validate_playback_accounting(
    snapshot: &LabSessionEvidenceSnapshot,
    completed_requests: &HashSet<u64>,
) -> Result<PlaybackAccounting, LabSessionAggregateError> {
    let playback_requests: HashSet<u64> = snapshot
        .voice_attempts
        .iter()
        .filter(|attempt| attempt.canonical_playback_confirmed)
        .map(|attempt| attempt.request_sequence)
        .collect();
    let audio_requests: HashSet<u64> = snapshot
        .media_events
        .iter()
        .filter(|event| event.kind == LabMediaEvidenceKind::AudioStarted)
        .filter_map(|event| event.request_sequence)
        .collect();
    let playback_completed_requests: HashSet<u64> = snapshot
        .media_events
        .iter()
        .filter(|event| event.kind == LabMediaEvidenceKind::PlaybackCompleted)
        .filter_map(|event| event.request_sequence)
        .collect();
    let interrupted_requests: HashSet<u64> = snapshot
        .media_events
        .iter()
        .filter(|event| event.kind == LabMediaEvidenceKind::InterruptionStopped)
        .filter_map(|event| event.request_sequence)
        .collect();

    if playback_requests != playback_completed_requests
        || !playback_requests.is_subset(&audio_requests)
        || !interrupted_requests.is_subset(&audio_requests)
        || !playback_requests.is_disjoint(&interrupted_requests)
    {
        return Err(LabSessionAggregateError::InvalidSnapshot);
    }

    let accounted_requests: HashSet<u64> = playback_requests
        .union(&interrupted_requests)
        .copied()
        .collect();
    let derived_playback = !completed_requests.is_empty()
        && !playback_requests.is_empty()
        && accounted_requests == *completed_requests;

    Ok(PlaybackAccounting {
        playback_requests,
        audio_requests,
        derived_playback,
    })
}
