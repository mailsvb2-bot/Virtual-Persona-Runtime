use std::{env, fs};

use serde::Serialize;
use vpr_evaluation::{
    BoundGoldenReport, GoldenEvidenceBundle, LiveProviderProbeReceipt, ProviderStateManifest,
    Rt0ExitEvidence, Rt0ExitVerificationContext, evaluate_rt0_exit_evidence,
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
    let mut args = env::args().skip(1);
    let Some(exit_evidence_path) = args.next() else {
        return usage();
    };
    let Some(golden_report_path) = args.next() else {
        return usage();
    };
    let Some(golden_evidence_path) = args.next() else {
        return usage();
    };
    let Some(provider_state_path) = args.next() else {
        return usage();
    };
    let Some(live_provider_probe_path) = args.next() else {
        return usage();
    };
    let Some(release_spec_path) = args.next() else {
        return usage();
    };
    let Some(candidate_sha) = args.next() else {
        return usage();
    };
    if args.next().is_some() {
        return usage();
    }

    let exit_evidence_bytes = read(&exit_evidence_path)?;
    let golden_report_bytes = read(&golden_report_path)?;
    let golden_evidence_bytes = read(&golden_evidence_path)?;
    let provider_state_bytes = read(&provider_state_path)?;
    let live_provider_probe_bytes = read(&live_provider_probe_path)?;
    let release_spec_bytes = read(&release_spec_path)?;
    let evidence: Rt0ExitEvidence = parse(&exit_evidence_bytes)?;
    let golden_report: BoundGoldenReport = parse(&golden_report_bytes)?;
    let golden_evidence_bundle: GoldenEvidenceBundle = parse(&golden_evidence_bytes)?;
    let provider_state: ProviderStateManifest = parse(&provider_state_bytes)?;
    let live_provider_probe: LiveProviderProbeReceipt = parse(&live_provider_probe_bytes)?;

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
            release_spec_bytes: &release_spec_bytes,
            exact_candidate_sha: &candidate_sha,
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
        "usage: vpr-rt0-exit-evidence <exit-evidence.json> <golden-report.json> <golden-evidence.json> <provider-state.json> <live-provider-probe.json> <release-spec.md> <exact-candidate-sha>"
    );
    Err(2)
}
