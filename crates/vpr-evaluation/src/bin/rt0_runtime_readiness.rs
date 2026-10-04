use std::{env, fs, process};

use serde::Serialize;
use vpr_evaluation::{
    BoundLabSessionEvidenceAggregate, LabAvSyncTrackIssue, LabSessionEvidenceSnapshot,
    LabVoiceAttemptStatus, LatencyDistributionMillis, ParticipantRole,
    bind_owner_lab_session_evidence, derive_rt0_runtime_supporting_projection,
    rt0_quality_failure_codes, sha256_hex, validate_rt0_conversation_attempt_artifact,
};

const REPORT_SCHEMA: &str = "rt0-runtime-readiness-0.1";

#[derive(Debug, Default, Serialize)]
#[serde(deny_unknown_fields)]
struct AvSyncIssueCounts {
    stats_unavailable: u32,
    timestamp_unavailable: u32,
    sender_report_timing_unavailable: u32,
    ambiguous_streams: u32,
    no_unique_active_stream: u32,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct RuntimeCostObservation {
    session_duration_millis: u64,
    estimated_cost_microunits: Option<u64>,
    provider_charge_microunits: Option<u64>,
    runtime_cost_signal_observed: bool,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct RuntimeMetricObservation {
    text_first_meaningful_response: Option<LatencyDistributionMillis>,
    first_meaningful_audio: Option<LatencyDistributionMillis>,
    interruption_stop: Option<LatencyDistributionMillis>,
    first_useful_video: Option<LatencyDistributionMillis>,
    av_sync_absolute_offset: Option<LatencyDistributionMillis>,
    recoverable_reconnect: Option<LatencyDistributionMillis>,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct RuntimeReadinessReport {
    schema_version: &'static str,
    candidate_sha: String,
    provider_state_sha256: String,
    session_snapshot_count: usize,
    owner_sessions: usize,
    visitor_sessions: usize,
    owner_completed_voice_attempts: usize,
    visitor_completed_voice_attempts: usize,
    canonical_playback_proven: bool,
    av_sync_proven: bool,
    av_sync_diagnostics: AvSyncIssueCounts,
    metrics: RuntimeMetricObservation,
    runtime_cost: RuntimeCostObservation,
    missing_runtime_evidence: Vec<&'static str>,
    runtime_supporting_projection_ready: bool,
    runtime_projection_error: Option<String>,
    quality_threshold_failures: Vec<vpr_evaluation::Rt0ExitFailureCode>,
    quality_thresholds_pass: bool,
    manual_cost_review_required: bool,
    privacy_acceptance_required: bool,
    human_quality_review_required: bool,
    participant_provenance_review_required: bool,
    known_limitations_review_required: bool,
    release_ready_claimed: bool,
}

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match run(&args) {
        Ok((report, ready)) => {
            match serde_json::to_string_pretty(&report) {
                Ok(json) => println!("{json}"),
                Err(_) => {
                    eprintln!("{{\"ok\":false,\"code\":\"OUTPUT_FAILED\"}}");
                    process::exit(2);
                }
            }
            if !ready {
                process::exit(1);
            }
        }
        Err(code) => {
            eprintln!("{{\"ok\":false,\"code\":\"{code}\"}}");
            process::exit(2);
        }
    }
}

fn run(args: &[String]) -> Result<(RuntimeReadinessReport, bool), &'static str> {
    if args.len() < 5 {
        return Err("INPUT_REQUIRED");
    }

    let provider_state_bytes = fs::read(&args[0]).map_err(|_| "PROVIDER_STATE_READ_FAILED")?;
    let conversation_attempt_bytes =
        fs::read(&args[1]).map_err(|_| "CONVERSATION_ATTEMPT_READ_FAILED")?;
    let bound_bytes = fs::read(&args[2]).map_err(|_| "BOUND_AGGREGATE_READ_FAILED")?;
    let exact_candidate_sha = args[3].trim();
    let bound: BoundLabSessionEvidenceAggregate =
        serde_json::from_slice(&bound_bytes).map_err(|_| "BOUND_AGGREGATE_INVALID")?;

    let snapshot_bytes = args[4..]
        .iter()
        .map(|path| fs::read(path).map_err(|_| "SESSION_SNAPSHOT_READ_FAILED"))
        .collect::<Result<Vec<_>, _>>()?;
    let snapshot_refs = snapshot_bytes.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let snapshots = snapshot_bytes
        .iter()
        .map(|bytes| {
            serde_json::from_slice::<LabSessionEvidenceSnapshot>(bytes)
                .map_err(|_| "SESSION_SNAPSHOT_INVALID")
        })
        .collect::<Result<Vec<_>, _>>()?;

    let provider_state_sha256 = sha256_hex(&provider_state_bytes);
    validate_rt0_conversation_attempt_artifact(
        &conversation_attempt_bytes,
        exact_candidate_sha,
        &provider_state_sha256,
    )
    .map_err(|_| "CONVERSATION_ATTEMPT_INVALID")?;

    let recomputed =
        bind_owner_lab_session_evidence(&snapshot_refs, &provider_state_bytes, exact_candidate_sha)
            .map_err(|_| "SESSION_BINDING_INVALID")?;
    if recomputed != bound {
        return Err("BOUND_AGGREGATE_MISMATCH");
    }

    let mut owner_sessions = 0usize;
    let mut visitor_sessions = 0usize;
    let mut owner_completed_voice_attempts = 0usize;
    let mut visitor_completed_voice_attempts = 0usize;
    let mut av_sync_diagnostics = AvSyncIssueCounts::default();

    for snapshot in &snapshots {
        let completed = snapshot
            .voice_attempts
            .iter()
            .filter(|attempt| attempt.status == LabVoiceAttemptStatus::Completed)
            .count();
        match snapshot.participant_role {
            ParticipantRole::Owner => {
                owner_sessions += 1;
                owner_completed_voice_attempts += completed;
            }
            ParticipantRole::Visitor => {
                visitor_sessions += 1;
                visitor_completed_voice_attempts += completed;
            }
        }
        for diagnostic in &snapshot.av_sync_diagnostics {
            if let Some(issue) = diagnostic.audio_issue {
                record_av_sync_issue(&mut av_sync_diagnostics, issue);
            }
            if let Some(issue) = diagnostic.video_issue {
                record_av_sync_issue(&mut av_sync_diagnostics, issue);
            }
        }
    }

    let aggregate = &bound.aggregate;
    let mut missing_runtime_evidence = Vec::new();
    if owner_sessions == 0 {
        missing_runtime_evidence.push("owner_session_snapshot");
    }
    if visitor_sessions == 0 {
        missing_runtime_evidence.push("visitor_session_snapshot");
    }
    if owner_completed_voice_attempts == 0 {
        missing_runtime_evidence.push("owner_completed_voice_attempt");
    }
    if visitor_completed_voice_attempts == 0 {
        missing_runtime_evidence.push("visitor_completed_voice_attempt");
    }
    if !aggregate.canonical_playback_proven {
        missing_runtime_evidence.push("canonical_playback");
    }
    if aggregate.text_first_meaningful_response.is_none() {
        missing_runtime_evidence.push("text_first_meaningful_response");
    }
    if aggregate.first_meaningful_audio.is_none() {
        missing_runtime_evidence.push("first_meaningful_audio");
    }
    if aggregate.interruption_stop.is_none() {
        missing_runtime_evidence.push("interruption_stop");
    }
    if aggregate.first_useful_video.is_none() {
        missing_runtime_evidence.push("first_useful_video");
    }
    if !aggregate.av_sync_proven || aggregate.av_sync_absolute_offset.is_none() {
        missing_runtime_evidence.push("av_sync");
    }
    if aggregate.recoverable_reconnect.is_none() {
        missing_runtime_evidence.push("recoverable_reconnect");
    }
    if aggregate.session_duration_millis == 0 {
        missing_runtime_evidence.push("session_duration");
    }

    let projection = derive_rt0_runtime_supporting_projection(
        &conversation_attempt_bytes,
        &bound,
        &snapshot_refs,
        &provider_state_bytes,
        exact_candidate_sha,
    );
    let (runtime_supporting_projection_ready, runtime_projection_error, quality_threshold_failures) =
        match projection {
            Ok(projection) => (true, None, rt0_quality_failure_codes(&projection.quality)),
            Err(error) => (false, Some(format!("{error:?}")), Vec::new()),
        };
    let quality_thresholds_pass =
        runtime_supporting_projection_ready && quality_threshold_failures.is_empty();
    let runtime_cost_signal_observed = aggregate.estimated_cost_microunits.is_some()
        || aggregate.provider_charge_microunits.is_some();

    let report = RuntimeReadinessReport {
        schema_version: REPORT_SCHEMA,
        candidate_sha: exact_candidate_sha.to_string(),
        provider_state_sha256,
        session_snapshot_count: snapshots.len(),
        owner_sessions,
        visitor_sessions,
        owner_completed_voice_attempts,
        visitor_completed_voice_attempts,
        canonical_playback_proven: aggregate.canonical_playback_proven,
        av_sync_proven: aggregate.av_sync_proven,
        av_sync_diagnostics,
        metrics: RuntimeMetricObservation {
            text_first_meaningful_response: aggregate.text_first_meaningful_response,
            first_meaningful_audio: aggregate.first_meaningful_audio,
            interruption_stop: aggregate.interruption_stop,
            first_useful_video: aggregate.first_useful_video,
            av_sync_absolute_offset: aggregate.av_sync_absolute_offset,
            recoverable_reconnect: aggregate.recoverable_reconnect,
        },
        runtime_cost: RuntimeCostObservation {
            session_duration_millis: aggregate.session_duration_millis,
            estimated_cost_microunits: aggregate.estimated_cost_microunits,
            provider_charge_microunits: aggregate.provider_charge_microunits,
            runtime_cost_signal_observed,
        },
        missing_runtime_evidence,
        runtime_supporting_projection_ready,
        runtime_projection_error,
        quality_threshold_failures,
        quality_thresholds_pass,
        manual_cost_review_required: true,
        privacy_acceptance_required: true,
        human_quality_review_required: true,
        participant_provenance_review_required: true,
        known_limitations_review_required: true,
        release_ready_claimed: false,
    };

    let ready = report.runtime_supporting_projection_ready && report.quality_thresholds_pass;
    Ok((report, ready))
}

fn record_av_sync_issue(counts: &mut AvSyncIssueCounts, issue: LabAvSyncTrackIssue) {
    match issue {
        LabAvSyncTrackIssue::StatsUnavailable => counts.stats_unavailable += 1,
        LabAvSyncTrackIssue::TimestampUnavailable => counts.timestamp_unavailable += 1,
        LabAvSyncTrackIssue::SenderReportTimingUnavailable => {
            counts.sender_report_timing_unavailable += 1;
        }
        LabAvSyncTrackIssue::AmbiguousStreams => counts.ambiguous_streams += 1,
        LabAvSyncTrackIssue::NoUniqueActiveStream => counts.no_unique_active_stream += 1,
    }
}
