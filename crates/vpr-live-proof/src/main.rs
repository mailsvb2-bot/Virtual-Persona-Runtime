use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use vpr_live_proof::{LiveProofPreflightError, preflight};

#[derive(Serialize)]
struct CliError {
    ok: bool,
    code: LiveProofPreflightError,
}

fn main() {
    if let Err(code) = run() {
        std::process::exit(code);
    }
}

fn run() -> Result<(), i32> {
    let mut args = env::args().skip(1);
    let Some(output_path) = args.next() else {
        eprintln!("usage: vpr-live-proof <provider-state-output.json>");
        return Err(2);
    };
    if args.next().is_some() {
        eprintln!("usage: vpr-live-proof <provider-state-output.json>");
        return Err(2);
    }
    let candidate = git_output(["rev-parse", "HEAD"])?;
    let worktree_root = PathBuf::from(git_output(["rev-parse", "--show-toplevel"])?.trim());
    let output_path = PathBuf::from(&output_path);
    if let Err(code) = validate_output_path(&output_path, &worktree_root) {
        emit_error(code);
        return Err(2);
    }
    let status = git_output(["status", "--porcelain", "--untracked-files=all"])?;
    let egress_authorized =
        env::var("VPR_LIVE_PROOF_ALLOW_EGRESS").is_ok_and(|value| value == "true");
    let receipt = match preflight(
        candidate.trim(),
        status.trim().is_empty(),
        egress_authorized,
    ) {
        Ok(receipt) => receipt,
        Err(code) => {
            emit_error(code);
            return Err(2);
        }
    };
    let provider_state = serde_json::to_vec_pretty(&receipt.provider_state).map_err(|_| 2)?;
    atomic_write(&output_path, &provider_state)?;
    let final_candidate = git_output(["rev-parse", "HEAD"])?;
    let final_status = git_output(["status", "--porcelain", "--untracked-files=all"])?;
    if final_candidate.trim() != candidate.trim() {
        let _ = fs::remove_file(&output_path);
        emit_error(LiveProofPreflightError::CandidateChanged);
        return Err(2);
    }
    if !final_status.trim().is_empty() {
        let _ = fs::remove_file(&output_path);
        emit_error(LiveProofPreflightError::WorktreeDirty);
        return Err(2);
    }
    println!("{}", serde_json::to_string_pretty(&receipt).map_err(|_| 2)?);
    Ok(())
}

fn validate_output_path(
    output_path: &Path,
    worktree_root: &Path,
) -> Result<(), LiveProofPreflightError> {
    if !output_path.is_absolute() {
        return Err(LiveProofPreflightError::OutputPathInvalid);
    }
    let root =
        fs::canonicalize(worktree_root).map_err(|_| LiveProofPreflightError::OutputPathInvalid)?;
    let parent = output_path
        .parent()
        .ok_or(LiveProofPreflightError::OutputPathInvalid)?;
    let parent =
        fs::canonicalize(parent).map_err(|_| LiveProofPreflightError::OutputPathInvalid)?;
    let file_name = output_path
        .file_name()
        .ok_or(LiveProofPreflightError::OutputPathInvalid)?;
    let resolved = parent.join(file_name);
    if resolved.starts_with(root) {
        return Err(LiveProofPreflightError::OutputPathInsideWorktree);
    }
    Ok(())
}

fn git_output<const N: usize>(args: [&str; N]) -> Result<String, i32> {
    let output = Command::new("git").args(args).output().map_err(|_| 2)?;
    if !output.status.success() {
        return Err(2);
    }
    String::from_utf8(output.stdout).map_err(|_| 2)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), i32> {
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let file_name = path.file_name().and_then(|value| value.to_str()).ok_or(2)?;
    let temp = parent.join(format!(".{file_name}.tmp-{}", std::process::id()));
    fs::write(&temp, bytes).map_err(|_| 2)?;
    fs::rename(&temp, path).map_err(|_| {
        let _ = fs::remove_file(&temp);
        2
    })
}

fn emit_error(code: LiveProofPreflightError) {
    eprintln!(
        "{}",
        serde_json::to_string(&CliError { ok: false, code })
            .unwrap_or_else(|_| "{\"ok\":false}".into())
    );
}
