use std::{env, fs};

use serde::Serialize;
use vpr_evaluation::{
    BoundGoldenReport, BoundLabSessionEvidenceAggregate, GoldenEvidenceBundle,
    LiveProviderProbeReceipt, ProviderStateManifest, Rt0ExitEvidence, Rt0ExitVerificationContext,
    evaluate_rt0_exit_evidence,
};

#[derive(Serialize)]
struct CliError<T: Serialize> {
    ok: bool,
    code: T,
}

fn main() {
    if let Err(code) = run() {
        std::process::exit(code);
    }
}

fn run() -> Result<(), i32> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 10 {
        return usage();
    }

    let exit_evidence_path = &args[0];
    let golden_report_path = &args[1];
    let golden_evidence_path = &args[2];
    let provider_state_path = &args[3];
    let live_provider_probe_path = &args[4];
    let conversation_attempt_path = &args[5];
    let bound_session_aggregate_path = &args[6];
    let snapshot_paths = &args[7..args.len() - 2];
    let release_spec_path = &args[args.len() - 2];
    let candidate_sha = &args[args.len() - 1];

    let exit_evidence_bytes = read(exit_evidence_path)?;
    let golden_report_bytes = read(golden_report_path)?;
    let golden_evidence_bytes = read(golden_evidence_path)?;
    let provider_state_bytes = read(provider_state_path)?;
    let live_provider_probe_bytes = read(live_provider_probe_path)?;
    let conversation_attempt_bytes = read(conversation_attempt_path)?;
    let bound_session_aggregate_bytes = read(bound_session_aggregate_path)?;
    let session_snapshot_bytes = snapshot_paths
        .iter()
        .map(|path| read(path))
        .collect::<Result<Vec<_>, _>>()?;
    let session_snapshot_artifacts: Vec<&[u8]> = session_snapshot_bytes
        .iter()
        .map(Vec::as_slice)
        .collect();
    let release_spec_bytes = read(release_spec_path)?;
    let evidence: Rt0ExitEvidence = parse(&exit_evidence_bytes)?;
    let golden_report: BoundGoldenReport = parse(&golden_report_bytes)?;
    let golden_evidence_bundle: GoldenEvidenceBundle = parse(&golden_evidence_bytes)?;
    let provider_state: ProviderStateManifest = parse(&provider_state_bytes)?;
    let live_provider_probe: LiveProviderProbeReceipt = parse(&live_provider_probe_bytes)?;
    let bound_session_aggregate: BoundLabSessionEvidenceAggregate =
        parse(&bound_session_aggregate_bytes)?;

    let report = match evaluate_rt0_exit_evidence(
        &evidence,
        &golden_report,
        Rt0ExitVerificationContext {
            exit_evidence_bytes: &exit_evidence_bytes,
            golden_report_bytes: &golden_report_bytes,
            golden_evidence_bundle: &golden_evidence_bundle,
            golden_evidence_bytes: &golden_evidence_bytes,
            provider_state: &provider_state,
            provider_state_bytes: &provider_state_bytes,
            live_provider_probe: &live_provider_probe,
            live_provider_probe_bytes: &live_provider_probe_bytes,
            conversation_attempt_bytes: &conversation_attempt_bytes,
            bound_session_aggregate: &bound_session_aggregate,
            bound_session_aggregate_bytes: &bound_session_aggregate_bytes,
            session_snapshot_artifacts: &session_snapshot_artifacts,
            release_spec_bytes: &release_spec_bytes,
            exact_candidate_sha: candidate_sha,
        },
    ) {
        Ok(report) => report,
        Err(code) => {
            emit_error(code)?;
            return Err(2);
        }
    };

    println!("{}", serde_json::to_string_pretty(&report).map_err(|_| 2)?);
    if report.ready { Ok(()) } else { Err(1) }
}

fn read(path: &str) -> Result<Vec<u8>, i32> {
    fs::read(path).map_err(|_| {
        let _ = emit_error("INPUT_INVALID");
        2
    })
}

fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, i32> {
    serde_json::from_slice(bytes).map_err(|_| {
        let _ = emit_error("INPUT_INVALID");
        2
    })
}

fn emit_error<T: Serialize>(code: T) -> Result<(), i32> {
    eprintln!(
        "{}",
        serde_json::to_string(&CliError { ok: false, code }).map_err(|_| 2)?
    );
    Ok(())
}

fn usage() -> Result<(), i32> {
    eprintln!(
        "usage: vpr-rt0-exit-evidence <exit-evidence.json> <golden-report.json> <golden-evidence.json> <provider-state.json> <live-provider-probe.json> <conversation-attempt.json> <bound-session-aggregate.json> <session-snapshot.json>... <release-spec.md> <exact-candidate-sha>"
    );
    Err(2)
}
