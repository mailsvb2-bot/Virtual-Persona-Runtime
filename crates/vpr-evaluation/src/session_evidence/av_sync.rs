use std::collections::{BTreeMap, HashSet};

use super::{
    LabAvSyncReference, LabMediaEvidenceKind, LabSessionAggregateError,
    LabSessionEvidenceSnapshot, LabVoiceAttemptStatus, MAX_MEDIA_ELAPSED_MILLIS,
    RT0_AV_SYNC_SAMPLES_PER_REQUEST,
};

pub(super) fn validate_and_collect_av_sync(
    snapshot: &LabSessionEvidenceSnapshot,
    request_status: &BTreeMap<u64, LabVoiceAttemptStatus>,
    playback_requests: &HashSet<u64>,
    offsets: &mut Vec<u64>,
) -> Result<HashSet<u64>, LabSessionAggregateError> {
    let mut unique = HashSet::new();
    let mut sequences_by_request: BTreeMap<u64, HashSet<u32>> = BTreeMap::new();
    let mut reference_by_request: BTreeMap<u64, LabAvSyncReference> = BTreeMap::new();
    let audio_requests: HashSet<u64> = snapshot
        .media_events
        .iter()
        .filter(|event| event.kind == LabMediaEvidenceKind::AudioStarted)
        .filter_map(|event| event.request_sequence)
        .collect();
    for sample in &snapshot.av_sync_samples {
        if sample.request_sequence == 0
            || !(1..=RT0_AV_SYNC_SAMPLES_PER_REQUEST).contains(&sample.sample_sequence)
            || sample.absolute_offset_millis > MAX_MEDIA_ELAPSED_MILLIS
            || request_status.get(&sample.request_sequence)
                != Some(&LabVoiceAttemptStatus::Completed)
            || !audio_requests.contains(&sample.request_sequence)
            || !unique.insert((sample.request_sequence, sample.sample_sequence))
        {
            return Err(LabSessionAggregateError::InvalidMediaEvidence);
        }
        if reference_by_request
            .insert(sample.request_sequence, sample.reference)
            .is_some_and(|reference| reference != sample.reference)
        {
            return Err(LabSessionAggregateError::InvalidMediaEvidence);
        }
        sequences_by_request
            .entry(sample.request_sequence)
            .or_default()
            .insert(sample.sample_sequence);
        if playback_requests.contains(&sample.request_sequence) {
            offsets.push(sample.absolute_offset_millis);
        }
    }
    let required_samples = usize::try_from(RT0_AV_SYNC_SAMPLES_PER_REQUEST)
        .map_err(|_| LabSessionAggregateError::Overflow)?;
    Ok(sequences_by_request
        .into_iter()
        .filter(|(request, sequences)| {
            playback_requests.contains(request) && sequences.len() == required_samples
        })
        .map(|(request, _)| request)
        .collect())
}
