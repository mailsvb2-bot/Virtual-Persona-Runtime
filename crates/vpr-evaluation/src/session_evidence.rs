use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

mod av_sync;
use av_sync::validate_and_collect_av_sync;
mod av_sync_diagnostic;
use av_sync_diagnostic::validate_av_sync_diagnostics;
pub use av_sync_diagnostic::{LabAvSyncDiagnostic, LabAvSyncDiagnosticInput, LabAvSyncTrackIssue};
mod playback;
use playback::validate_playback_accounting;
mod snapshot_header;
use snapshot_header::validate_snapshot_header;

use crate::session_statistics::{add_cost, distribution};
use crate::{
    LabTextAttemptEvidence, LabTextAttemptStatus, LatencyDistributionMillis, ParticipantRole,
    SessionUsageEvidence, sha256_hex,
};

pub const RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA: &str = "rt0-owner-lab-session-evidence-1.3";
pub const RT0_OWNER_LAB_SESSION_AGGREGATE_SCHEMA: &str = "rt0-owner-lab-session-aggregate-1.0";
pub const RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE: &str = "browser_observed_media_plane_only";
pub const RT0_AV_SYNC_SAMPLES_PER_REQUEST: u32 = 3;
const MAX_MEDIA_ELAPSED_MILLIS: u64 = 300_000;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LabMediaEvidenceKind {
    VideoReady,
    BackendStartReady,
    TransportConnectStarted,
    TransportConnected,
    RemoteVideoTrackReceived,
    RemoteVideoAttached,
    EndToEndVideoReady,
    BackendCompleteReceived,
    ClientDeliverySent,
    AudioStarted,
    ProviderDataReceived,
    ProviderPlaybackDoneReceived,
    PlaybackRecoveryTriggered,
    PlaybackCompleted,
    InterruptionStopped,
    ReconnectRestored,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LabMediaEvidenceInput {
    pub session_sequence: u64,
    pub request_sequence: Option<u64>,
    pub kind: LabMediaEvidenceKind,
    pub elapsed_millis: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LabAvSyncReference {
    WebRtcEstimatedPlayoutTimestamp,
    HtmlMediaElementCurrentTime,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LabAvSyncEvidenceInput {
    pub session_sequence: u64,
    pub request_sequence: u64,
    pub sample_sequence: u32,
    pub reference: LabAvSyncReference,
    pub absolute_offset_millis: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LabAvSyncEvidence {
    pub request_sequence: u64,
    pub sample_sequence: u32,
    pub reference: LabAvSyncReference,
    pub absolute_offset_millis: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LabVoiceAttemptStatus {
    Pending,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LabVoiceAttemptEvidence {
    pub request_sequence: u64,
    pub canonical_turn_sequence: Option<u64>,
    pub canonical_output_sequence: Option<u64>,
    pub canonical_playback_confirmed: bool,
    pub status: LabVoiceAttemptStatus,
    pub failure_code: Option<String>,
    pub stt_millis: Option<u64>,
    pub llm_millis: Option<u64>,
    pub llm_first_meaningful_millis: Option<u64>,
    pub avatar_millis: Option<u64>,
    pub server_total_millis: Option<u64>,
    pub stt_usage: Option<SessionUsageEvidence>,
    pub llm_usage: Option<SessionUsageEvidence>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LabMediaEvidence {
    pub request_sequence: Option<u64>,
    pub kind: LabMediaEvidenceKind,
    pub elapsed_millis: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LabSessionEvidenceSnapshot {
    pub schema_version: String,
    pub candidate_sha: String,
    pub provider_state_sha256: String,
    pub scope: String,
    pub session_sequence: u64,
    pub participant_role: ParticipantRole,
    pub session_duration_millis: u64,
    pub canonical_playback_proven: bool,
    pub av_sync_proven: bool,
    pub text_attempts: Vec<LabTextAttemptEvidence>,
    pub voice_attempts: Vec<LabVoiceAttemptEvidence>,
    pub media_events: Vec<LabMediaEvidence>,
    pub av_sync_samples: Vec<LabAvSyncEvidence>,
    #[serde(default)]
    pub av_sync_diagnostics: Vec<LabAvSyncDiagnostic>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LabSessionEvidenceAggregate {
    pub schema_version: String,
    pub source_schema_version: String,
    pub sessions: u32,
    pub session_duration_millis: u64,
    pub completed_text_attempts: u32,
    pub failed_text_attempts: u32,
    pub completed_voice_attempts: u32,
    pub failed_voice_attempts: u32,
    pub canonical_playback_proven: bool,
    pub av_sync_proven: bool,
    pub text_first_meaningful_response: Option<LatencyDistributionMillis>,
    pub av_sync_absolute_offset: Option<LatencyDistributionMillis>,
    pub stt_latency: Option<LatencyDistributionMillis>,
    pub llm_latency: Option<LatencyDistributionMillis>,
    pub llm_first_meaningful_response: Option<LatencyDistributionMillis>,
    pub avatar_submit_latency: Option<LatencyDistributionMillis>,
    pub server_total_latency: Option<LatencyDistributionMillis>,
    pub first_meaningful_audio: Option<LatencyDistributionMillis>,
    pub interruption_stop: Option<LatencyDistributionMillis>,
    pub first_useful_video: Option<LatencyDistributionMillis>,
    pub backend_start_ready: Option<LatencyDistributionMillis>,
    pub transport_connect_started: Option<LatencyDistributionMillis>,
    pub transport_connected: Option<LatencyDistributionMillis>,
    pub remote_video_track_received: Option<LatencyDistributionMillis>,
    pub remote_video_attached: Option<LatencyDistributionMillis>,
    pub end_to_end_first_useful_video: Option<LatencyDistributionMillis>,
    pub recoverable_reconnect: Option<LatencyDistributionMillis>,
    pub estimated_cost_microunits: Option<u64>,
    pub provider_charge_microunits: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabSessionAggregateError {
    EmptyInput,
    InvalidSnapshot,
    DuplicateSnapshot,
    IncompleteAttempt,
    InvalidMediaEvidence,
    Overflow,
}

/// Aggregates sanitized Owner Lab session snapshots into deterministic RT0 evidence.
///
/// # Errors
///
/// Returns an error when input is empty, duplicated, incomplete, structurally invalid,
/// contains invalid media evidence, or would overflow aggregate counters.
pub fn aggregate_owner_lab_session_evidence(
    snapshots: &[LabSessionEvidenceSnapshot],
) -> Result<LabSessionEvidenceAggregate, LabSessionAggregateError> {
    if snapshots.is_empty() {
        return Err(LabSessionAggregateError::EmptyInput);
    }
    let sessions =
        u32::try_from(snapshots.len()).map_err(|_| LabSessionAggregateError::Overflow)?;
    let mut digests = HashSet::new();
    let mut session_sequences = HashSet::new();
    let mut accumulator = SessionAggregateAccumulator::default();
    for snapshot in snapshots {
        validate_snapshot_header(snapshot)?;
        if !session_sequences.insert(snapshot.session_sequence) {
            return Err(LabSessionAggregateError::DuplicateSnapshot);
        }
        let encoded =
            serde_json::to_vec(snapshot).map_err(|_| LabSessionAggregateError::InvalidSnapshot)?;
        if !digests.insert(sha256_hex(&encoded)) {
            return Err(LabSessionAggregateError::DuplicateSnapshot);
        }
        accumulator.consume_snapshot(snapshot)?;
    }
    accumulator.finish(sessions)
}

#[derive(Default)]
struct SessionAggregateAccumulator {
    session_duration_millis: u64,
    completed_text: u32,
    failed_text: u32,
    completed_voice: u32,
    failed_voice: u32,
    text_first_meaningful: Vec<u64>,
    playback_sessions: u32,
    av_sync_sessions: u32,
    av_sync: Vec<u64>,
    stt: Vec<u64>,
    llm: Vec<u64>,
    llm_first_meaningful: Vec<u64>,
    avatar: Vec<u64>,
    server_total: Vec<u64>,
    audio: Vec<u64>,
    interruption: Vec<u64>,
    video: Vec<u64>,
    backend_start: Vec<u64>,
    transport_connect_started: Vec<u64>,
    transport_connected: Vec<u64>,
    remote_video_track_received: Vec<u64>,
    remote_video_attached: Vec<u64>,
    end_to_end_video: Vec<u64>,
    reconnect: Vec<u64>,
    estimated_cost: Option<u64>,
    provider_charge: Option<u64>,
    cost_initialized: bool,
}

impl SessionAggregateAccumulator {
    fn consume_snapshot(
        &mut self,
        snapshot: &LabSessionEvidenceSnapshot,
    ) -> Result<(), LabSessionAggregateError> {
        self.session_duration_millis = self
            .session_duration_millis
            .checked_add(snapshot.session_duration_millis)
            .ok_or(LabSessionAggregateError::Overflow)?;
        let mut text_request_sequences = HashSet::new();
        for attempt in &snapshot.text_attempts {
            if attempt.request_sequence == 0
                || !text_request_sequences.insert(attempt.request_sequence)
            {
                return Err(LabSessionAggregateError::InvalidSnapshot);
            }
            self.consume_text_attempt(attempt)?;
        }

        let mut request_status = BTreeMap::new();
        for attempt in &snapshot.voice_attempts {
            if attempt.request_sequence == 0
                || request_status
                    .insert(attempt.request_sequence, attempt.status)
                    .is_some()
            {
                return Err(LabSessionAggregateError::InvalidSnapshot);
            }
            self.consume_attempt(attempt)?;
        }
        validate_and_collect_media(snapshot, &request_status, self)?;
        let completed_requests: HashSet<u64> = snapshot
            .voice_attempts
            .iter()
            .filter(|attempt| attempt.status == LabVoiceAttemptStatus::Completed)
            .map(|attempt| attempt.request_sequence)
            .collect();
        let playback = validate_playback_accounting(snapshot, &completed_requests)?;
        if snapshot.canonical_playback_proven != playback.derived_playback {
            return Err(LabSessionAggregateError::InvalidSnapshot);
        }
        if playback.derived_playback {
            self.playback_sessions = self
                .playback_sessions
                .checked_add(1)
                .ok_or(LabSessionAggregateError::Overflow)?;
        }
        let av_sync_requests = validate_and_collect_av_sync(
            snapshot,
            &request_status,
            &playback.playback_requests,
            &mut self.av_sync,
        )?;
        validate_av_sync_diagnostics(
            snapshot,
            &request_status,
            &playback.audio_requests,
            &av_sync_requests,
        )?;
        let derived_av_sync =
            playback.derived_playback && av_sync_requests == playback.playback_requests;
        if snapshot.av_sync_proven != derived_av_sync {
            return Err(LabSessionAggregateError::InvalidSnapshot);
        }
        if derived_av_sync {
            self.av_sync_sessions = self
                .av_sync_sessions
                .checked_add(1)
                .ok_or(LabSessionAggregateError::Overflow)?;
        }
        Ok(())
    }

    fn consume_text_attempt(
        &mut self,
        attempt: &LabTextAttemptEvidence,
    ) -> Result<(), LabSessionAggregateError> {
        match attempt.status {
            LabTextAttemptStatus::Pending => Err(LabSessionAggregateError::IncompleteAttempt),
            LabTextAttemptStatus::Failed => {
                if attempt
                    .failure_code
                    .as_deref()
                    .is_none_or(|code| code.trim().is_empty())
                    || attempt.canonical_turn_sequence.is_some()
                    || attempt.canonical_output_sequence.is_some()
                    || attempt.first_meaningful_response_millis.is_some()
                    || attempt.server_total_millis.is_some()
                    || attempt.llm_usage.is_some()
                {
                    return Err(LabSessionAggregateError::InvalidSnapshot);
                }
                self.failed_text = self
                    .failed_text
                    .checked_add(1)
                    .ok_or(LabSessionAggregateError::Overflow)?;
                Ok(())
            }
            LabTextAttemptStatus::Completed => {
                let (Some(turn), Some(output), Some(first), Some(total), Some(llm_usage)) = (
                    attempt.canonical_turn_sequence,
                    attempt.canonical_output_sequence,
                    attempt.first_meaningful_response_millis,
                    attempt.server_total_millis,
                    attempt.llm_usage.as_ref(),
                ) else {
                    return Err(LabSessionAggregateError::IncompleteAttempt);
                };
                if turn == 0 || output == 0 || first > total || attempt.failure_code.is_some() {
                    return Err(LabSessionAggregateError::InvalidSnapshot);
                }
                self.completed_text = self
                    .completed_text
                    .checked_add(1)
                    .ok_or(LabSessionAggregateError::Overflow)?;
                self.text_first_meaningful.push(first);
                self.add_single_usage_cost(llm_usage)
            }
        }
    }

    fn consume_attempt(
        &mut self,
        attempt: &LabVoiceAttemptEvidence,
    ) -> Result<(), LabSessionAggregateError> {
        match attempt.status {
            LabVoiceAttemptStatus::Pending => Err(LabSessionAggregateError::IncompleteAttempt),
            LabVoiceAttemptStatus::Failed => self.consume_failed_attempt(attempt),
            LabVoiceAttemptStatus::Completed => self.consume_completed_attempt(attempt),
        }
    }

    fn consume_failed_attempt(
        &mut self,
        attempt: &LabVoiceAttemptEvidence,
    ) -> Result<(), LabSessionAggregateError> {
        if attempt
            .failure_code
            .as_deref()
            .is_none_or(|code| code.trim().is_empty())
            || attempt.canonical_turn_sequence.is_some()
            || attempt.canonical_output_sequence.is_some()
            || attempt.canonical_playback_confirmed
            || attempt.stt_millis.is_some()
            || attempt.llm_millis.is_some()
            || attempt.llm_first_meaningful_millis.is_some()
            || attempt.avatar_millis.is_some()
            || attempt.server_total_millis.is_some()
            || attempt.stt_usage.is_some()
            || attempt.llm_usage.is_some()
        {
            return Err(LabSessionAggregateError::InvalidSnapshot);
        }
        self.failed_voice = self
            .failed_voice
            .checked_add(1)
            .ok_or(LabSessionAggregateError::Overflow)?;
        Ok(())
    }

    fn consume_completed_attempt(
        &mut self,
        attempt: &LabVoiceAttemptEvidence,
    ) -> Result<(), LabSessionAggregateError> {
        let (
            Some(turn),
            Some(output),
            Some(stt_ms),
            Some(llm_ms),
            Some(llm_first_meaningful_ms),
            Some(avatar_ms),
            Some(total_ms),
            Some(stt_usage),
            Some(llm_usage),
        ) = (
            attempt.canonical_turn_sequence,
            attempt.canonical_output_sequence,
            attempt.stt_millis,
            attempt.llm_millis,
            attempt.llm_first_meaningful_millis,
            attempt.avatar_millis,
            attempt.server_total_millis,
            attempt.stt_usage.as_ref(),
            attempt.llm_usage.as_ref(),
        )
        else {
            return Err(LabSessionAggregateError::IncompleteAttempt);
        };
        if turn == 0
            || output == 0
            || llm_first_meaningful_ms > llm_ms
            || llm_first_meaningful_ms > total_ms
            || attempt.failure_code.is_some()
        {
            return Err(LabSessionAggregateError::InvalidSnapshot);
        }
        self.completed_voice = self
            .completed_voice
            .checked_add(1)
            .ok_or(LabSessionAggregateError::Overflow)?;
        self.stt.push(stt_ms);
        self.llm.push(llm_ms);
        self.llm_first_meaningful.push(llm_first_meaningful_ms);
        self.avatar.push(avatar_ms);
        self.server_total.push(total_ms);
        self.add_usage_costs(stt_usage, llm_usage)
    }

    fn add_single_usage_cost(
        &mut self,
        usage: &SessionUsageEvidence,
    ) -> Result<(), LabSessionAggregateError> {
        if !self.cost_initialized {
            self.estimated_cost = Some(0);
            self.provider_charge = Some(0);
            self.cost_initialized = true;
        }
        add_cost(&mut self.estimated_cost, usage.estimated_cost_microunits)?;
        add_cost(&mut self.provider_charge, usage.provider_charge_microunits)
    }

    fn add_usage_costs(
        &mut self,
        stt: &SessionUsageEvidence,
        llm: &SessionUsageEvidence,
    ) -> Result<(), LabSessionAggregateError> {
        if !self.cost_initialized {
            self.estimated_cost = Some(0);
            self.provider_charge = Some(0);
            self.cost_initialized = true;
        }
        add_cost(&mut self.estimated_cost, stt.estimated_cost_microunits)?;
        add_cost(&mut self.estimated_cost, llm.estimated_cost_microunits)?;
        add_cost(&mut self.provider_charge, stt.provider_charge_microunits)?;
        add_cost(&mut self.provider_charge, llm.provider_charge_microunits)
    }

    fn finish(
        self,
        sessions: u32,
    ) -> Result<LabSessionEvidenceAggregate, LabSessionAggregateError> {
        Ok(LabSessionEvidenceAggregate {
            schema_version: RT0_OWNER_LAB_SESSION_AGGREGATE_SCHEMA.into(),
            source_schema_version: RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA.into(),
            sessions,
            session_duration_millis: self.session_duration_millis,
            completed_text_attempts: self.completed_text,
            failed_text_attempts: self.failed_text,
            completed_voice_attempts: self.completed_voice,
            failed_voice_attempts: self.failed_voice,
            canonical_playback_proven: self.completed_voice > 0
                && self.playback_sessions == sessions,
            av_sync_proven: self.completed_voice > 0 && self.av_sync_sessions == sessions,
            text_first_meaningful_response: distribution(self.text_first_meaningful)?,
            av_sync_absolute_offset: distribution(self.av_sync)?,
            stt_latency: distribution(self.stt)?,
            llm_latency: distribution(self.llm)?,
            llm_first_meaningful_response: distribution(self.llm_first_meaningful)?,
            avatar_submit_latency: distribution(self.avatar)?,
            server_total_latency: distribution(self.server_total)?,
            first_meaningful_audio: distribution(self.audio)?,
            interruption_stop: distribution(self.interruption)?,
            first_useful_video: distribution(self.video)?,
            backend_start_ready: distribution(self.backend_start)?,
            transport_connect_started: distribution(self.transport_connect_started)?,
            transport_connected: distribution(self.transport_connected)?,
            remote_video_track_received: distribution(self.remote_video_track_received)?,
            remote_video_attached: distribution(self.remote_video_attached)?,
            end_to_end_first_useful_video: distribution(self.end_to_end_video)?,
            recoverable_reconnect: distribution(self.reconnect)?,
            estimated_cost_microunits: self.estimated_cost,
            provider_charge_microunits: self.provider_charge,
        })
    }
}

fn validate_and_collect_media(
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
            | LabMediaEvidenceKind::ProviderPlaybackDoneReceived
            | LabMediaEvidenceKind::PlaybackRecoveryTriggered
            | LabMediaEvidenceKind::BackendCompleteReceived
            | LabMediaEvidenceKind::ClientDeliverySent => {}
        }
    }
    Ok(())
}
