use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{LatencyDistributionMillis, ParticipantRole, sha256_hex};

pub const RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA: &str = "rt0-owner-lab-session-evidence-0.4";
pub const RT0_OWNER_LAB_SESSION_AGGREGATE_SCHEMA: &str = "rt0-owner-lab-session-aggregate-0.4";
pub const RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE: &str = "browser_observed_media_plane_only";
pub const RT0_AV_SYNC_SAMPLES_PER_REQUEST: u32 = 3;
const MAX_MEDIA_ELAPSED_MILLIS: u64 = 300_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SessionUsageEvidence {
    pub input_units: Option<u64>,
    pub output_units: Option<u64>,
    pub estimated_cost_microunits: Option<u64>,
    pub provider_charge_microunits: Option<u64>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LabMediaEvidenceKind {
    VideoReady,
    AudioStarted,
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
    pub scope: String,
    pub session_sequence: u64,
    pub participant_role: ParticipantRole,
    pub canonical_playback_proven: bool,
    pub av_sync_proven: bool,
    pub voice_attempts: Vec<LabVoiceAttemptEvidence>,
    pub media_events: Vec<LabMediaEvidence>,
    pub av_sync_samples: Vec<LabAvSyncEvidence>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LabSessionEvidenceAggregate {
    pub schema_version: String,
    pub source_schema_version: String,
    pub sessions: u32,
    pub completed_voice_attempts: u32,
    pub failed_voice_attempts: u32,
    pub canonical_playback_proven: bool,
    pub av_sync_proven: bool,
    pub av_sync_absolute_offset: Option<LatencyDistributionMillis>,
    pub stt_latency: Option<LatencyDistributionMillis>,
    pub llm_latency: Option<LatencyDistributionMillis>,
    pub avatar_submit_latency: Option<LatencyDistributionMillis>,
    pub server_total_latency: Option<LatencyDistributionMillis>,
    pub first_meaningful_audio: Option<LatencyDistributionMillis>,
    pub interruption_stop: Option<LatencyDistributionMillis>,
    pub first_useful_video: Option<LatencyDistributionMillis>,
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

/// Aggregates sanitized Owner Lab session snapshots into deterministic latency/cost evidence.
///
/// # Errors
/// Returns a fail-closed error for empty input, malformed or duplicate snapshots, pending or
/// inconsistent voice attempts, malformed media evidence, or arithmetic overflow.
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
    completed: u32,
    failed: u32,
    playback_sessions: u32,
    av_sync_sessions: u32,
    av_sync: Vec<u64>,
    stt: Vec<u64>,
    llm: Vec<u64>,
    avatar: Vec<u64>,
    server_total: Vec<u64>,
    audio: Vec<u64>,
    interruption: Vec<u64>,
    video: Vec<u64>,
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
        validate_and_collect_media(
            snapshot,
            &request_status,
            &mut self.audio,
            &mut self.interruption,
            &mut self.video,
            &mut self.reconnect,
        )?;
        let completed_requests: HashSet<u64> = snapshot
            .voice_attempts
            .iter()
            .filter(|attempt| attempt.status == LabVoiceAttemptStatus::Completed)
            .map(|attempt| attempt.request_sequence)
            .collect();
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
        if playback_requests != audio_requests {
            return Err(LabSessionAggregateError::InvalidSnapshot);
        }
        let derived_playback =
            !completed_requests.is_empty() && playback_requests == completed_requests;
        if snapshot.canonical_playback_proven != derived_playback {
            return Err(LabSessionAggregateError::InvalidSnapshot);
        }
        if derived_playback {
            self.playback_sessions = self
                .playback_sessions
                .checked_add(1)
                .ok_or(LabSessionAggregateError::Overflow)?;
        }
        let av_sync_requests = validate_and_collect_av_sync(
            snapshot,
            &request_status,
            &playback_requests,
            &mut self.av_sync,
        )?;
        let derived_av_sync = derived_playback && av_sync_requests == completed_requests;
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
            || attempt.avatar_millis.is_some()
            || attempt.server_total_millis.is_some()
            || attempt.stt_usage.is_some()
            || attempt.llm_usage.is_some()
        {
            return Err(LabSessionAggregateError::InvalidSnapshot);
        }
        self.failed = self
            .failed
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
            Some(avatar_ms),
            Some(total_ms),
            Some(stt_usage),
            Some(llm_usage),
        ) = (
            attempt.canonical_turn_sequence,
            attempt.canonical_output_sequence,
            attempt.stt_millis,
            attempt.llm_millis,
            attempt.avatar_millis,
            attempt.server_total_millis,
            attempt.stt_usage.as_ref(),
            attempt.llm_usage.as_ref(),
        )
        else {
            return Err(LabSessionAggregateError::IncompleteAttempt);
        };
        if turn == 0 || output == 0 || attempt.failure_code.is_some() {
            return Err(LabSessionAggregateError::InvalidSnapshot);
        }
        self.completed = self
            .completed
            .checked_add(1)
            .ok_or(LabSessionAggregateError::Overflow)?;
        self.stt.push(stt_ms);
        self.llm.push(llm_ms);
        self.avatar.push(avatar_ms);
        self.server_total.push(total_ms);
        self.add_usage_costs(stt_usage, llm_usage)
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
            completed_voice_attempts: self.completed,
            failed_voice_attempts: self.failed,
            canonical_playback_proven: self.completed > 0 && self.playback_sessions == sessions,
            av_sync_proven: self.completed > 0 && self.av_sync_sessions == sessions,
            av_sync_absolute_offset: distribution(self.av_sync)?,
            stt_latency: distribution(self.stt)?,
            llm_latency: distribution(self.llm)?,
            avatar_submit_latency: distribution(self.avatar)?,
            server_total_latency: distribution(self.server_total)?,
            first_meaningful_audio: distribution(self.audio)?,
            interruption_stop: distribution(self.interruption)?,
            first_useful_video: distribution(self.video)?,
            recoverable_reconnect: distribution(self.reconnect)?,
            estimated_cost_microunits: self.estimated_cost,
            provider_charge_microunits: self.provider_charge,
        })
    }
}

fn validate_snapshot_header(
    snapshot: &LabSessionEvidenceSnapshot,
) -> Result<(), LabSessionAggregateError> {
    if snapshot.schema_version != RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA
        || snapshot.scope != RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE
        || snapshot.session_sequence == 0
    {
        return Err(LabSessionAggregateError::InvalidSnapshot);
    }
    Ok(())
}

fn validate_and_collect_media(
    snapshot: &LabSessionEvidenceSnapshot,
    request_status: &BTreeMap<u64, LabVoiceAttemptStatus>,
    audio: &mut Vec<u64>,
    interruption: &mut Vec<u64>,
    video: &mut Vec<u64>,
    reconnect: &mut Vec<u64>,
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
            LabMediaEvidenceKind::AudioStarted | LabMediaEvidenceKind::InterruptionStopped
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
            LabMediaEvidenceKind::AudioStarted => audio.push(event.elapsed_millis),
            LabMediaEvidenceKind::InterruptionStopped => interruption.push(event.elapsed_millis),
            LabMediaEvidenceKind::VideoReady => video.push(event.elapsed_millis),
            LabMediaEvidenceKind::ReconnectRestored => reconnect.push(event.elapsed_millis),
        }
    }
    Ok(())
}

fn validate_and_collect_av_sync(
    snapshot: &LabSessionEvidenceSnapshot,
    request_status: &BTreeMap<u64, LabVoiceAttemptStatus>,
    playback_requests: &HashSet<u64>,
    offsets: &mut Vec<u64>,
) -> Result<HashSet<u64>, LabSessionAggregateError> {
    let mut unique = HashSet::new();
    let mut sequences_by_request: BTreeMap<u64, HashSet<u32>> = BTreeMap::new();
    for sample in &snapshot.av_sync_samples {
        if sample.request_sequence == 0
            || !(1..=RT0_AV_SYNC_SAMPLES_PER_REQUEST).contains(&sample.sample_sequence)
            || sample.absolute_offset_millis > MAX_MEDIA_ELAPSED_MILLIS
            || request_status.get(&sample.request_sequence)
                != Some(&LabVoiceAttemptStatus::Completed)
            || !playback_requests.contains(&sample.request_sequence)
            || !unique.insert((sample.request_sequence, sample.sample_sequence))
        {
            return Err(LabSessionAggregateError::InvalidMediaEvidence);
        }
        sequences_by_request
            .entry(sample.request_sequence)
            .or_default()
            .insert(sample.sample_sequence);
        offsets.push(sample.absolute_offset_millis);
    }
    let required_samples = usize::try_from(RT0_AV_SYNC_SAMPLES_PER_REQUEST)
        .map_err(|_| LabSessionAggregateError::Overflow)?;
    Ok(sequences_by_request
        .into_iter()
        .filter(|(_, sequences)| sequences.len() == required_samples)
        .map(|(request, _)| request)
        .collect())
}

fn add_cost(total: &mut Option<u64>, value: Option<u64>) -> Result<(), LabSessionAggregateError> {
    let (Some(current), Some(value)) = (*total, value) else {
        *total = None;
        return Ok(());
    };
    *total = Some(
        current
            .checked_add(value)
            .ok_or(LabSessionAggregateError::Overflow)?,
    );
    Ok(())
}

fn distribution(
    mut values: Vec<u64>,
) -> Result<Option<LatencyDistributionMillis>, LabSessionAggregateError> {
    if values.is_empty() {
        return Ok(None);
    }
    values.sort_unstable();
    let samples = u32::try_from(values.len()).map_err(|_| LabSessionAggregateError::Overflow)?;
    Ok(Some(LatencyDistributionMillis {
        samples,
        p50: nearest_rank(&values, 50),
        p95: nearest_rank(&values, 95),
    }))
}

fn nearest_rank(values: &[u64], percentile: usize) -> u64 {
    let rank = values.len().saturating_mul(percentile).div_ceil(100).max(1);
    values[rank - 1]
}
