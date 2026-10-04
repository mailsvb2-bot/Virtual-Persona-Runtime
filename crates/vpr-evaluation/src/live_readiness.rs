use serde::Serialize;

use crate::exit_checks::{
    RT0_AV_SYNC_P95_MAX_MILLIS, RT0_FIRST_MEANINGFUL_AUDIO_P50_MAX_MILLIS,
    RT0_FIRST_MEANINGFUL_AUDIO_P95_MAX_MILLIS, RT0_FIRST_USEFUL_VIDEO_P95_MAX_MILLIS,
    RT0_INTERRUPTION_STOP_P95_MAX_MILLIS, RT0_RECOVERABLE_RECONNECT_P95_MAX_MILLIS,
    RT0_TEXT_FIRST_MEANINGFUL_P50_MAX_MILLIS, RT0_TEXT_FIRST_MEANINGFUL_P95_MAX_MILLIS,
};
use crate::{BoundLabSessionEvidenceAggregate, LatencyDistributionMillis};

pub const RT0_LIVE_READINESS_SCHEMA: &str = "rt0-live-readiness-report-0.1";

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Rt0LiveReadinessBlocker {
    CanonicalPlaybackNotProven,
    TextLatencyMissing,
    TextLatencyExceeded,
    FirstAudioMissing,
    FirstAudioExceeded,
    InterruptionStopMissing,
    InterruptionStopExceeded,
    FirstUsefulVideoMissing,
    FirstUsefulVideoExceeded,
    AvSyncNotProven,
    AvSyncMissing,
    AvSyncExceeded,
    ReconnectMissing,
    ReconnectExceeded,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Rt0LatencyReadiness {
    pub available: bool,
    pub samples: Option<u32>,
    pub p50_millis: Option<u64>,
    pub p95_millis: Option<u64>,
    pub max_p50_millis: Option<u64>,
    pub max_p95_millis: u64,
    pub passes_threshold: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Rt0QualityReadiness {
    pub text_first_meaningful_response: Rt0LatencyReadiness,
    pub first_meaningful_audio: Rt0LatencyReadiness,
    pub interruption_stop: Rt0LatencyReadiness,
    pub first_useful_video: Rt0LatencyReadiness,
    pub av_sync_absolute_offset: Rt0LatencyReadiness,
    pub recoverable_reconnect: Rt0LatencyReadiness,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Rt0RuntimeProofFlags {
    pub canonical_playback_proven: bool,
    pub av_sync_proven: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Rt0ReadinessDecisionFlags {
    pub complete_provider_cost_review_still_required: bool,
    pub runtime_quality_ready: bool,
    pub rerun_required: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Rt0LiveReadinessReport {
    pub schema_version: String,
    pub candidate_sha: String,
    pub provider_state_sha256: String,
    pub sessions: u32,
    pub session_duration_millis: u64,
    pub completed_text_attempts: u32,
    pub failed_text_attempts: u32,
    pub completed_voice_attempts: u32,
    pub failed_voice_attempts: u32,
    #[serde(flatten)]
    pub proof: Rt0RuntimeProofFlags,
    pub estimated_cost_microunits: Option<u64>,
    pub provider_charge_microunits: Option<u64>,
    #[serde(flatten)]
    pub decision: Rt0ReadinessDecisionFlags,
    pub quality: Rt0QualityReadiness,
    pub blockers: Vec<Rt0LiveReadinessBlocker>,
}

#[must_use]
pub fn derive_rt0_live_readiness(
    bound: &BoundLabSessionEvidenceAggregate,
) -> Rt0LiveReadinessReport {
    let aggregate = &bound.aggregate;
    let quality = Rt0QualityReadiness {
        text_first_meaningful_response: latency_readiness(
            aggregate.text_first_meaningful_response,
            Some(RT0_TEXT_FIRST_MEANINGFUL_P50_MAX_MILLIS),
            RT0_TEXT_FIRST_MEANINGFUL_P95_MAX_MILLIS,
        ),
        first_meaningful_audio: latency_readiness(
            aggregate.first_meaningful_audio,
            Some(RT0_FIRST_MEANINGFUL_AUDIO_P50_MAX_MILLIS),
            RT0_FIRST_MEANINGFUL_AUDIO_P95_MAX_MILLIS,
        ),
        interruption_stop: latency_readiness(
            aggregate.interruption_stop,
            None,
            RT0_INTERRUPTION_STOP_P95_MAX_MILLIS,
        ),
        first_useful_video: latency_readiness(
            aggregate.first_useful_video,
            None,
            RT0_FIRST_USEFUL_VIDEO_P95_MAX_MILLIS,
        ),
        av_sync_absolute_offset: latency_readiness(
            aggregate.av_sync_absolute_offset,
            None,
            RT0_AV_SYNC_P95_MAX_MILLIS,
        ),
        recoverable_reconnect: latency_readiness(
            aggregate.recoverable_reconnect,
            None,
            RT0_RECOVERABLE_RECONNECT_P95_MAX_MILLIS,
        ),
    };

    let mut blockers = Vec::new();
    if !aggregate.canonical_playback_proven {
        blockers.push(Rt0LiveReadinessBlocker::CanonicalPlaybackNotProven);
    }
    push_latency_blocker(
        &mut blockers,
        &quality.text_first_meaningful_response,
        Rt0LiveReadinessBlocker::TextLatencyMissing,
        Rt0LiveReadinessBlocker::TextLatencyExceeded,
    );
    push_latency_blocker(
        &mut blockers,
        &quality.first_meaningful_audio,
        Rt0LiveReadinessBlocker::FirstAudioMissing,
        Rt0LiveReadinessBlocker::FirstAudioExceeded,
    );
    push_latency_blocker(
        &mut blockers,
        &quality.interruption_stop,
        Rt0LiveReadinessBlocker::InterruptionStopMissing,
        Rt0LiveReadinessBlocker::InterruptionStopExceeded,
    );
    push_latency_blocker(
        &mut blockers,
        &quality.first_useful_video,
        Rt0LiveReadinessBlocker::FirstUsefulVideoMissing,
        Rt0LiveReadinessBlocker::FirstUsefulVideoExceeded,
    );
    if !aggregate.av_sync_proven {
        blockers.push(Rt0LiveReadinessBlocker::AvSyncNotProven);
    }
    push_latency_blocker(
        &mut blockers,
        &quality.av_sync_absolute_offset,
        Rt0LiveReadinessBlocker::AvSyncMissing,
        Rt0LiveReadinessBlocker::AvSyncExceeded,
    );
    push_latency_blocker(
        &mut blockers,
        &quality.recoverable_reconnect,
        Rt0LiveReadinessBlocker::ReconnectMissing,
        Rt0LiveReadinessBlocker::ReconnectExceeded,
    );

    let runtime_quality_ready = blockers.is_empty();

    Rt0LiveReadinessReport {
        schema_version: RT0_LIVE_READINESS_SCHEMA.into(),
        candidate_sha: bound.candidate_sha.clone(),
        provider_state_sha256: bound.provider_state_sha256.clone(),
        sessions: aggregate.sessions,
        session_duration_millis: aggregate.session_duration_millis,
        completed_text_attempts: aggregate.completed_text_attempts,
        failed_text_attempts: aggregate.failed_text_attempts,
        completed_voice_attempts: aggregate.completed_voice_attempts,
        failed_voice_attempts: aggregate.failed_voice_attempts,
        proof: Rt0RuntimeProofFlags {
            canonical_playback_proven: aggregate.canonical_playback_proven,
            av_sync_proven: aggregate.av_sync_proven,
        },
        estimated_cost_microunits: aggregate.estimated_cost_microunits,
        provider_charge_microunits: aggregate.provider_charge_microunits,
        decision: Rt0ReadinessDecisionFlags {
            complete_provider_cost_review_still_required: true,
            runtime_quality_ready,
            rerun_required: !runtime_quality_ready,
        },
        quality,
        blockers,
    }
}

fn latency_readiness(
    distribution: Option<LatencyDistributionMillis>,
    max_p50_millis: Option<u64>,
    max_p95_millis: u64,
) -> Rt0LatencyReadiness {
    let Some(distribution) = distribution else {
        return Rt0LatencyReadiness {
            available: false,
            samples: None,
            p50_millis: None,
            p95_millis: None,
            max_p50_millis,
            max_p95_millis,
            passes_threshold: false,
        };
    };
    let p50_passes = max_p50_millis.is_none_or(|limit| distribution.p50 <= limit);
    Rt0LatencyReadiness {
        available: true,
        samples: Some(distribution.samples),
        p50_millis: Some(distribution.p50),
        p95_millis: Some(distribution.p95),
        max_p50_millis,
        max_p95_millis,
        passes_threshold: distribution.samples > 0
            && distribution.p50 <= distribution.p95
            && p50_passes
            && distribution.p95 <= max_p95_millis,
    }
}

fn push_latency_blocker(
    blockers: &mut Vec<Rt0LiveReadinessBlocker>,
    metric: &Rt0LatencyReadiness,
    missing: Rt0LiveReadinessBlocker,
    exceeded: Rt0LiveReadinessBlocker,
) {
    if !metric.available {
        blockers.push(missing);
    } else if !metric.passes_threshold {
        blockers.push(exceeded);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        LabSessionEvidenceAggregate, RT0_OWNER_LAB_SESSION_AGGREGATE_SCHEMA,
        RT0_OWNER_LAB_SESSION_BINDING_SCHEMA, RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA,
    };

    fn distribution(p50: u64, p95: u64) -> LatencyDistributionMillis {
        LatencyDistributionMillis {
            samples: 3,
            p50,
            p95,
        }
    }

    fn passing_bound() -> BoundLabSessionEvidenceAggregate {
        BoundLabSessionEvidenceAggregate {
            schema_version: RT0_OWNER_LAB_SESSION_BINDING_SCHEMA.into(),
            candidate_sha: "a".repeat(40),
            provider_state_sha256: "b".repeat(64),
            snapshot_sha256: vec!["c".repeat(64)],
            aggregate: LabSessionEvidenceAggregate {
                schema_version: RT0_OWNER_LAB_SESSION_AGGREGATE_SCHEMA.into(),
                source_schema_version: RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA.into(),
                sessions: 1,
                session_duration_millis: 30_000,
                completed_text_attempts: 1,
                failed_text_attempts: 0,
                completed_voice_attempts: 1,
                failed_voice_attempts: 0,
                canonical_playback_proven: true,
                av_sync_proven: true,
                text_first_meaningful_response: Some(distribution(1_000, 2_500)),
                av_sync_absolute_offset: Some(distribution(90, 120)),
                stt_latency: None,
                llm_latency: None,
                llm_first_meaningful_response: None,
                avatar_submit_latency: None,
                server_total_latency: None,
                first_meaningful_audio: Some(distribution(1_500, 3_000)),
                interruption_stop: Some(distribution(300, 500)),
                first_useful_video: Some(distribution(2_000, 2_500)),
                recoverable_reconnect: Some(distribution(4_000, 5_000)),
                estimated_cost_microunits: Some(10),
                provider_charge_microunits: None,
            },
        }
    }

    #[test]
    fn exact_release_spec_boundaries_are_ready() {
        let report = derive_rt0_live_readiness(&passing_bound());
        assert!(report.decision.runtime_quality_ready);
        assert!(!report.decision.rerun_required);
        assert!(report.blockers.is_empty());
        assert!(report.quality.first_meaningful_audio.passes_threshold);
    }

    #[test]
    fn missing_reconnect_is_reported_without_promoting_readiness() {
        let mut bound = passing_bound();
        bound.aggregate.recoverable_reconnect = None;
        let report = derive_rt0_live_readiness(&bound);
        assert!(!report.decision.runtime_quality_ready);
        assert!(report.decision.rerun_required);
        assert!(
            report
                .blockers
                .contains(&Rt0LiveReadinessBlocker::ReconnectMissing)
        );
    }

    #[test]
    fn live_audio_above_contract_is_reported() {
        let mut bound = passing_bound();
        bound.aggregate.first_meaningful_audio = Some(distribution(1_501, 2_900));
        let report = derive_rt0_live_readiness(&bound);
        assert!(
            report
                .blockers
                .contains(&Rt0LiveReadinessBlocker::FirstAudioExceeded)
        );
    }

    #[test]
    fn av_sync_samples_do_not_count_when_full_proof_is_missing() {
        let mut bound = passing_bound();
        bound.aggregate.av_sync_proven = false;
        let report = derive_rt0_live_readiness(&bound);
        assert!(
            report
                .blockers
                .contains(&Rt0LiveReadinessBlocker::AvSyncNotProven)
        );
        assert!(!report.decision.runtime_quality_ready);
    }
}
