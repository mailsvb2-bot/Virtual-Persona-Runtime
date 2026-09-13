use std::{env, fs};

use serde::Serialize;
use vpr_evaluation::{
    EvidenceBindingError, EvidenceVerificationContext, GoldenEvidenceBundle, GoldenSuite,
    ProviderStateManifest, evaluate_bound_golden_suite,
};

#[derive(Serialize)]
struct CliError {
    ok: bool,
    code: EvidenceBindingError,
}

fn main() {
    if let Err(code) = run() {
        std::process::exit(code);
    }
}

fn run() -> Result<(), i32> {
    let mut args = env::args().skip(1);
    let Some(suite_path) = args.next() else {
        return usage();
    };
    let Some(evidence_path) = args.next() else {
        return usage();
    };
    let Some(release_spec_path) = args.next() else {
        return usage();
    };
    let Some(provider_state_path) = args.next() else {
        return usage();
    };
    let Some(candidate_sha) = args.next() else {
        return usage();
    };
    if args.next().is_some() {
        return usage();
    }

    let suite_bytes = fs::read(suite_path).map_err(|_| 2)?;
    let evidence_bytes = fs::read(evidence_path).map_err(|_| 2)?;
    let release_spec_bytes = fs::read(release_spec_path).map_err(|_| 2)?;
    let provider_state_bytes = fs::read(provider_state_path).map_err(|_| 2)?;
    let suite: GoldenSuite = serde_json::from_slice(&suite_bytes).map_err(|_| 2)?;
    let bundle: GoldenEvidenceBundle = serde_json::from_slice(&evidence_bytes).map_err(|_| 2)?;
    let provider_state: ProviderStateManifest =
        serde_json::from_slice(&provider_state_bytes).map_err(|_| 2)?;
    let report = match evaluate_bound_golden_suite(
        &suite,
        &bundle,
        EvidenceVerificationContext {
            suite_bytes: &suite_bytes,
            release_spec_bytes: &release_spec_bytes,
            provider_state: &provider_state,
            provider_state_bytes: &provider_state_bytes,
            evidence_bytes: &evidence_bytes,
            exact_candidate_sha: &candidate_sha,
        },
    ) {
        Ok(report) => report,
        Err(code) => {
            eprintln!(
                "{}",
                serde_json::to_string(&CliError { ok: false, code }).map_err(|_| 2)?
            );
            return Err(2);
        }
    };
    println!("{}", serde_json::to_string_pretty(&report).map_err(|_| 2)?);
    if report.golden.failed == 0 {
        Ok(())
    } else {
        Err(1)
    }
}

fn usage() -> Result<(), i32> {
    eprintln!(
        "usage: vpr-evaluation <suite.json> <evidence.json> <release-spec.md> <provider-state.json> <exact-candidate-sha>"
    );
    Err(2)
}
