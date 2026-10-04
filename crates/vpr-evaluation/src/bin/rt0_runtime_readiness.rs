use std::{env, fs, process};

use serde::Serialize;
use vpr_evaluation::{
    BoundLabSessionEvidenceAggregate, CheckStatus, LabAvSyncTrackIssue,
    LabSessionEvidenceAggregate, LabSessionEvidenceSnapshot, LabVoiceAttemptStatus,
    LatencyDistributionMillis, ParticipantRole, Rt0ExitFailureCode,
    bind_owner_lab_session_evidence, derive_rt0_runtime_supporting_projection,
    rt0_quality_failure_codes, sha256_hex, validate_rt0_conversation_attempt_artifact,
};

const REPORT_SCHEMA: &str = "rt0-runtime-readiness-0.1";
const NON_RUNTIME_REQUIREMENTS: [&str; 8] = [
    "automated_ci_e2e",
    "cost_review",
    "privacy_permissions",
    "acceptance_paths",
    "golden_evidence",
    "human_quality_review",
    "participant_provenance_review",
    "known_limitations_review",
];

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

#[derive(Debug, Default)]
struct SnapshotSummary {
    owner_sessions: usize,
    visitor_sessions: usize,
    owner_completed_voice_attempts: usize,
    visitor_completed_voice_attempts: usize,
    av_sync_diagnostics: AvSyncIssueCounts,
}

#[derive(Debug)]
struct ProjectionEvaluation {
    projection_status: CheckStatus,
    projection_error: Option<String>,
    quality_failures: Vec<Rt0ExitFailureCode>,
    quality_status: CheckStatus,
}

struct RuntimeInputs {
    provider_state_bytes: Vec<u8>,
    conversation_attempt_bytes: Vec<u8>,
    bound: BoundLabSessionEvidenceAggregate,
    exact_candidate_sha: String,
    snapshot_bytes: Vec<Vec<u8>>,
    snapshots: Vec<LabSessionEvidenceSnapshot>,
}

impl RuntimeInputs {
    fn snapshot_refs(&self) -> Vec<&[u8]> {
        self.snapshot_bytes.iter().map(Vec::as_slice).collect()
    }
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
    canonical_playback: CheckStatus,
    av_sync: CheckStatus,
    av_sync_diagnostics: AvSyncIssueCounts,
    metrics: RuntimeMetricObservation,
    runtime_cost: RuntimeCostObservation,
    missing_runtime_evidence: Vec<&'static str>,
    runtime_supporting_projection: CheckStatus,
    runtime_projection_error: Option<String>,
    quality_threshold_failures: Vec<Rt0ExitFailureCode>,
    quality_thresholds: CheckStatus,
    remaining_non_runtime_requirements: Vec<&'static str>,
    release_ready_claimed: bool,
}

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match run(&args) {
        Ok((report, ready)) => {
            let Ok(json) = serde_json::to_string_pretty(&report) else {
                eprintln!("{{\"ok\":false,\"code\":\"OUTPUT_FAILED\"}}");
                process::exit(2);
            };
            println!("{json}");
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
    let inputs = load_inputs(args)?;
    let provider_state_sha256 = validate_exact_binding(&inputs)?;
    let summary = summarize_snapshots(&inputs.snapshots);
    let aggregate = &inputs.bound.aggregate;
    let missing_runtime_evidence = missing_runtime_evidence(&summary, aggregate);
    let snapshot_refs = inputs.snapshot_refs();
    let projection = evaluate_projection(&inputs, &snapshot_refs);

    let ready = missing_runtime_evidence.is_empty()
        && projection.projection_status == CheckStatus::Passed
        && projection.quality_status == CheckStatus::Passed;
    let report = build_report(
        &inputs,
        provider_state_sha256,
        summary,
        missing_runtime_evidence,
        projection,
    );
    Ok((report, ready))
}

fn load_inputs(args: &[String]) -> Result<RuntimeInputs, &'static str> {
    if args.len() < 5 {
        return Err("INPUT_REQUIRED");
    }
    let provider_state_bytes = fs::read(&args[0]).map_err(|_| "PROVIDER_STATE_READ_FAILED")?;
    let conversation_attempt_bytes =
        fs::read(&args[1]).map_err(|_| "CONVERSATION_ATTEMPT_READ_FAILED")?;
    let bound_bytes = fs::read(&args[2]).map_err(|_| "BOUND_AGGREGATE_READ_FAILED")?;
    let bound = serde_json::from_slice(&bound_bytes).map_err(|_| "BOUND_AGGREGATE_INVALID")?;
    let snapshot_bytes = args[4..]
        .iter()
        .map(|path| fs::read(path).map_err(|_| "SESSION_SNAPSHOT_READ_FAILED"))
        .collect::<Result<Vec<_>, _>>()?;
    let snapshots = snapshot_bytes
        .iter()
        .map(|bytes| serde_json::from_slice(bytes).map_err(|_| "SESSION_SNAPSHOT_INVALID"))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RuntimeInputs {
        provider_state_bytes,
        conversation_attempt_bytes,
        bound,
        exact_candidate_sha: args[3].trim().to_string(),
        snapshot_bytes,
        snapshots,
    })
}

fn validate_exact_binding(inputs: &RuntimeInputs) -> Result<String, &'static str> {
    let provider_state_sha256 = sha256_hex(&inputs.provider_state_bytes);
    validate_rt0_conversation_attempt_artifact(
        &inputs.conversation_attempt_bytes,
        &inputs.exact_candidate_sha,
        &provider_state_sha256,
    )
    .map_err(|_| "CONVERSATION_ATTEMPT_INVALID")?;

    let snapshot_refs = inputs.snapshot_refs();
    let recomputed = bind_owner_lab_session_evidence(
        &snapshot_refs,
        &inputs.provider_state_bytes,
        &inputs.exact_candidate_sha,
    )
    .map_err(|_| "SESSION_BINDING_INVALID")?;
    if recomputed != inputs.bound {
        return Err("BOUND_AGGREGATE_MISMATCH");
    }
    Ok(provider_state_sha256)
}

fn summarize_snapshots(snapshots: &[LabSessionEvidenceSnapshot]) -> SnapshotSummary {
    let mut summary = SnapshotSummary::default();
    for snapshot in snapshots {
        let completed = snapshot
            .voice_attempts
            .iter()
            .filter(|attempt| attempt.status == LabVoiceAttemptStatus::Completed)
            .count();
        match snapshot.participant_role {
            ParticipantRole::Owner => {
                summary.owner_sessions += 1;
                summary.owner_completed_voice_attempts += completed;
            }
            ParticipantRole::Visitor => {
                summary.visitor_sessions += 1;
                summary.visitor_completed_voice_attempts += completed;
            }
        }
        collect_av_sync_diagnostics(&mut summary.av_sync_diagnostics, snapshot);
    }
    summary
}

fn collect_av_sync_diagnostics(
    counts: &mut AvSyncIssueCounts,
    snapshot: &LabSessionEvidenceSnapshot,
) {
    for diagnostic in &snapshot.av_sync_diagnostics {
        if let Some(issue) = diagnostic.audio_issue {
            record_av_sync_issue(counts, issue);
        }
        if let Some(issue) = diagnostic.video_issue {
            record_av_sync_issue(counts, issue);
        }
    }
}

fn missing_runtime_evidence(
    summary: &SnapshotSummary,
    aggregate: &LabSessionEvidenceAggregate,
) -> Vec<&'static str> {
    let mut missing = Vec::new();
    push_missing(
        summary.owner_sessions == 0,
        "owner_session_snapshot",
        &mut missing,
    );
    push_missing(
        summary.visitor_sessions == 0,
        "visitor_session_snapshot",
        &mut missing,
    );
    push_missing(
        summary.owner_completed_voice_attempts == 0,
        "owner_completed_voice_attempt",
        &mut missing,
    );
    push_missing(
        summary.visitor_completed_voice_attempts == 0,
        "visitor_completed_voice_attempt",
        &mut missing,
    );
    push_missing(
        !aggregate.canonical_playback_proven,
        "canonical_playback",
        &mut missing,
    );
    push_missing(
        aggregate.text_first_meaningful_response.is_none(),
        "text_first_meaningful_response",
        &mut missing,
    );
    push_missing(
        aggregate.first_meaningful_audio.is_none(),
        "first_meaningful_audio",
        &mut missing,
    );
    push_missing(
        aggregate.interruption_stop.is_none(),
        "interruption_stop",
        &mut missing,
    );
    push_missing(
        aggregate.first_useful_video.is_none(),
        "first_useful_video",
        &mut missing,
    );
    push_missing(
        !aggregate.av_sync_proven || aggregate.av_sync_absolute_offset.is_none(),
        "av_sync",
        &mut missing,
    );
    push_missing(
        aggregate.recoverable_reconnect.is_none(),
        "recoverable_reconnect",
        &mut missing,
    );
    push_missing(
        aggregate.session_duration_millis == 0,
        "session_duration",
        &mut missing,
    );
    missing
}

fn push_missing(condition: bool, name: &'static str, missing: &mut Vec<&'static str>) {
    if condition {
        missing.push(name);
    }
}

fn evaluate_projection(inputs: &RuntimeInputs, snapshot_refs: &[&[u8]]) -> ProjectionEvaluation {
    match derive_rt0_runtime_supporting_projection(
        &inputs.conversation_attempt_bytes,
        &inputs.bound,
        snapshot_refs,
        &inputs.provider_state_bytes,
        &inputs.exact_candidate_sha,
    ) {
        Ok(projection) => {
            let quality_failures = rt0_quality_failure_codes(&projection.quality);
            ProjectionEvaluation {
                projection_status: CheckStatus::Passed,
                projection_error: None,
                quality_status: status(quality_failures.is_empty()),
                quality_failures,
            }
        }
        Err(error) => ProjectionEvaluation {
            projection_status: CheckStatus::Failed,
            projection_error: Some(format!("{error:?}")),
            quality_failures: Vec::new(),
            quality_status: CheckStatus::Failed,
        },
    }
}

fn build_report(
    inputs: &RuntimeInputs,
    provider_state_sha256: String,
    summary: SnapshotSummary,
    missing_runtime_evidence: Vec<&'static str>,
    projection: ProjectionEvaluation,
) -> RuntimeReadinessReport {
    let aggregate = &inputs.bound.aggregate;
    RuntimeReadinessReport {
        schema_version: REPORT_SCHEMA,
        candidate_sha: inputs.exact_candidate_sha.clone(),
        provider_state_sha256,
        session_snapshot_count: inputs.snapshots.len(),
        owner_sessions: summary.owner_sessions,
        visitor_sessions: summary.visitor_sessions,
        owner_completed_voice_attempts: summary.owner_completed_voice_attempts,
        visitor_completed_voice_attempts: summary.visitor_completed_voice_attempts,
        canonical_playback: status(aggregate.canonical_playback_proven),
        av_sync: status(aggregate.av_sync_proven),
        av_sync_diagnostics: summary.av_sync_diagnostics,
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
            runtime_cost_signal_observed: aggregate.estimated_cost_microunits.is_some()
                || aggregate.provider_charge_microunits.is_some(),
        },
        missing_runtime_evidence,
        runtime_supporting_projection: projection.projection_status,
        runtime_projection_error: projection.projection_error,
        quality_threshold_failures: projection.quality_failures,
        quality_thresholds: projection.quality_status,
        remaining_non_runtime_requirements: NON_RUNTIME_REQUIREMENTS.to_vec(),
        release_ready_claimed: false,
    }
}

fn status(passed: bool) -> CheckStatus {
    if passed {
        CheckStatus::Passed
    } else {
        CheckStatus::Failed
    }
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
