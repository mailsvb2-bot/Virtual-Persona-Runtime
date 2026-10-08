use std::collections::{BTreeMap, HashSet};

use super::{
    LabMediaEvidenceKind, LabSessionAggregateError, LabSessionEvidenceSnapshot,
    LabVoiceAttemptStatus, MAX_MEDIA_ELAPSED_MILLIS, SessionAggregateAccumulator,
};

pub(super) fn validate_and_collect_media(
    snapshot: &LabSessionEvidenceSnapshot,
    request_status: &BTreeMap<u64, LabVoiceAttemptStatus>,
    accumulator: &mut SessionAggregateAccumulator,
) -> Result<(), LabSessionAggregateError> {
    let mut unique = HashSet::new();
    let audio_requests: HashSet<u64> = snapshot
        .media_events
        .iter()
        .filter(|event| event.kind == LabMediaEvidenceKind::AudioStarted)
        .filter_map(|event| event.request_sequence)
        .collect();
    for event in &snapshot.media_events {
        if event.elapsed_millis > MAX_MEDIA_ELAPSED_MILLIS {
            return Err(LabSessionAggregateError::InvalidMediaEvidence);
        }
        let requires_request = matches!(
            event.kind,
            LabMediaEvidenceKind::BackendCompleteReceived
                | LabMediaEvidenceKind::ClientDeliverySent
                | LabMediaEvidenceKind::AudioStarted
                | LabMediaEvidenceKind::ProviderDataReceived
                | LabMediaEvidenceKind::ProviderEventIgnored
                | LabMediaEvidenceKind::ProviderUnknownChatEvent
                | LabMediaEvidenceKind::ProviderUnknownVideoEvent
                | LabMediaEvidenceKind::ProviderUnknownToolEvent
                | LabMediaEvidenceKind::ProviderUnknownOtherEvent
                | LabMediaEvidenceKind::ProviderVideoGenerationStarted
                | LabMediaEvidenceKind::ProviderVideoGenerationDone
                | LabMediaEvidenceKind::ProviderVideoGenerationFailed
                | LabMediaEvidenceKind::ProviderInformationalEvent
                | LabMediaEvidenceKind::ProviderEventParseFailed
                | LabMediaEvidenceKind::ProviderPlaybackDoneReceived
                | LabMediaEvidenceKind::PlaybackRecoveryTriggered
                | LabMediaEvidenceKind::PlaybackCompleted
                | LabMediaEvidenceKind::InterruptionStopped
        );
        if requires_request != event.request_sequence.is_some() {
            return Err(LabSessionAggregateError::InvalidMediaEvidence);
        }
        if let Some(request) = event.request_sequence {
            if request_status.get(&request) != Some(&LabVoiceAttemptStatus::Completed) {
                return Err(LabSessionAggregateError::InvalidMediaEvidence);
            }
            if event.kind == LabMediaEvidenceKind::InterruptionStopped
                && !audio_requests.contains(&request)
            {
                return Err(LabSessionAggregateError::InvalidMediaEvidence);
            }
        }
        let key = (event.request_sequence, event.kind);
        if event.kind != LabMediaEvidenceKind::ReconnectRestored && !unique.insert(key) {
            return Err(LabSessionAggregateError::InvalidMediaEvidence);
        }
        match event.kind {
            LabMediaEvidenceKind::AudioStarted => accumulator.audio.push(event.elapsed_millis),
            LabMediaEvidenceKind::InterruptionStopped => {
                accumulator.interruption.push(event.elapsed_millis);
            }
            LabMediaEvidenceKind::VideoReady => accumulator.video.push(event.elapsed_millis),
            LabMediaEvidenceKind::BackendStartReady => {
                accumulator.backend_start.push(event.elapsed_millis);
            }
            LabMediaEvidenceKind::TransportConnectStarted => {
                accumulator
                    .transport_connect_started
                    .push(event.elapsed_millis);
            }
            LabMediaEvidenceKind::TransportConnected => {
                accumulator.transport_connected.push(event.elapsed_millis);
            }
            LabMediaEvidenceKind::RemoteVideoTrackReceived => {
                accumulator
                    .remote_video_track_received
                    .push(event.elapsed_millis);
            }
            LabMediaEvidenceKind::RemoteVideoAttached => {
                accumulator.remote_video_attached.push(event.elapsed_millis);
            }
            LabMediaEvidenceKind::EndToEndVideoReady => {
                accumulator.end_to_end_video.push(event.elapsed_millis);
            }
            LabMediaEvidenceKind::ReconnectRestored => {
                accumulator.reconnect.push(event.elapsed_millis);
            }
            LabMediaEvidenceKind::PlaybackCompleted
            | LabMediaEvidenceKind::ProviderDataReceived
            | LabMediaEvidenceKind::ProviderEventIgnored
            | LabMediaEvidenceKind::ProviderUnknownChatEvent
            | LabMediaEvidenceKind::ProviderUnknownVideoEvent
            | LabMediaEvidenceKind::ProviderUnknownToolEvent
            | LabMediaEvidenceKind::ProviderUnknownOtherEvent
            | LabMediaEvidenceKind::ProviderVideoGenerationStarted
            | LabMediaEvidenceKind::ProviderVideoGenerationDone
            | LabMediaEvidenceKind::ProviderVideoGenerationFailed
            | LabMediaEvidenceKind::ProviderInformationalEvent
            | LabMediaEvidenceKind::ProviderEventParseFailed
            | LabMediaEvidenceKind::ProviderPlaybackDoneReceived
            | LabMediaEvidenceKind::PlaybackRecoveryTriggered
            | LabMediaEvidenceKind::BackendCompleteReceived
            | LabMediaEvidenceKind::ClientDeliverySent => {}
        }
    }
    Ok(())
}
