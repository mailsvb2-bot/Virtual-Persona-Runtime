use std::{env, fs, process::ExitCode};

use vpr_evaluation::{
    LabSessionEvidenceSnapshot, aggregate_owner_lab_session_evidence,
    bind_owner_lab_session_evidence,
};

fn main() -> ExitCode {
    match run() {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(code) => {
            eprintln!("session-evidence aggregate failed: {code}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<String, &'static str> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.is_empty() {
        return Err("INPUT_REQUIRED");
    }
    if args.first().is_some_and(|arg| arg == "bind") {
        return run_bound(&args[1..]);
    }
    run_aggregate(&args)
}

fn run_aggregate(paths: &[std::ffi::OsString]) -> Result<String, &'static str> {
    let mut snapshots = Vec::with_capacity(paths.len());
    for path in paths {
        let bytes = fs::read(path).map_err(|_| "INPUT_READ_FAILED")?;
        let snapshot: LabSessionEvidenceSnapshot =
            serde_json::from_slice(&bytes).map_err(|_| "INPUT_INVALID")?;
        snapshots.push(snapshot);
    }
    let aggregate =
        aggregate_owner_lab_session_evidence(&snapshots).map_err(|_| "EVIDENCE_INVALID")?;
    serde_json::to_string_pretty(&aggregate).map_err(|_| "OUTPUT_FAILED")
}

fn run_bound(args: &[std::ffi::OsString]) -> Result<String, &'static str> {
    if args.len() < 3 {
        return Err("BIND_INPUT_REQUIRED");
    }
    let provider_state_bytes = fs::read(&args[0]).map_err(|_| "PROVIDER_STATE_READ_FAILED")?;
    let candidate_sha = args[1].to_str().ok_or("CANDIDATE_INVALID")?;
    let mut artifact_bytes = Vec::with_capacity(args.len() - 2);
    for path in &args[2..] {
        artifact_bytes.push(fs::read(path).map_err(|_| "INPUT_READ_FAILED")?);
    }
    let refs: Vec<&[u8]> = artifact_bytes.iter().map(Vec::as_slice).collect();
    let bound = bind_owner_lab_session_evidence(&refs, &provider_state_bytes, candidate_sha)
        .map_err(|_| "BINDING_INVALID")?;
    serde_json::to_string_pretty(&bound).map_err(|_| "OUTPUT_FAILED")
}
