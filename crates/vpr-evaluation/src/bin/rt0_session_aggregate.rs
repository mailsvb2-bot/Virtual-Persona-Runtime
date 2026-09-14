use std::{env, fs, process::ExitCode};

use vpr_evaluation::{LabSessionEvidenceSnapshot, aggregate_owner_lab_session_evidence};

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
    let paths: Vec<_> = env::args_os().skip(1).collect();
    if paths.is_empty() {
        return Err("INPUT_REQUIRED");
    }
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
