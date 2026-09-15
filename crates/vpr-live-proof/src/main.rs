use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use vpr_live_proof::{
    LiveConversationAttemptError, LiveProofPreflightError, LiveProviderProbeError, preflight,
    prepare, run_live_conversation_attempt, run_provider_probe,
};

#[derive(Serialize)]
struct CliError<'a> {
    ok: bool,
    code: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    stage: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
enum BoundaryError {
    InputPathInvalid,
    InputPathInsideWorktree,
    OutputPathInvalid,
    OutputPathInsideWorktree,
    OutputPathsConflict,
    InputReadFailed,
    ArtifactWriteFailed,
}

impl BoundaryError {
    const fn code(self) -> &'static str {
        match self {
            Self::InputPathInvalid => "INPUT_PATH_INVALID",
            Self::InputPathInsideWorktree => "INPUT_PATH_INSIDE_WORKTREE",
            Self::OutputPathInvalid => "OUTPUT_PATH_INVALID",
            Self::OutputPathInsideWorktree => "OUTPUT_PATH_INSIDE_WORKTREE",
            Self::OutputPathsConflict => "OUTPUT_PATHS_CONFLICT",
            Self::InputReadFailed => "INPUT_READ_FAILED",
            Self::ArtifactWriteFailed => "ARTIFACT_WRITE_FAILED",
        }
    }
}

struct RepoSnapshot {
    candidate: String,
    root: PathBuf,
}

fn main() {
    if let Err(code) = run() {
        std::process::exit(code);
    }
}

fn run() -> Result<(), i32> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.as_slice() {
        [provider_state_output] => run_preflight(Path::new(provider_state_output)),
        [mode, audio_input, provider_state_output, probe_output] if mode == "probe" => run_probe(
            Path::new(audio_input),
            Path::new(provider_state_output),
            Path::new(probe_output),
        ),
        [
            mode,
            profile_input,
            owner_audio,
            visitor_audio,
            provider_state_output,
            receipt_output,
        ] if mode == "conversation" => run_conversation(
            Path::new(profile_input),
            Path::new(owner_audio),
            Path::new(visitor_audio),
            Path::new(provider_state_output),
            Path::new(receipt_output),
        ),
        _ => {
            eprintln!(
                "usage: vpr-live-proof <provider-state-output.json>\n       vpr-live-proof probe <pcm-s16le-mono-16khz.raw> <provider-state-output.json> <probe-output.json>\n       vpr-live-proof conversation <reviewed-profile.json> <owner.raw> <visitor.raw> <provider-state-output.json> <conversation-receipt.json>"
            );
            Err(2)
        }
    }
}

fn run_preflight(output_path: &Path) -> Result<(), i32> {
    let snapshot = repo_snapshot()?;
    let output_path = validated_output_path(output_path, &snapshot.root).map_err(emit_boundary)?;
    let clean = worktree_clean()?;
    let receipt =
        preflight(&snapshot.candidate, clean, egress_authorized()).map_err(emit_preflight)?;
    let provider_state = serde_json::to_vec_pretty(&receipt.provider_state).map_err(|_| 2)?;
    atomic_write(&output_path, &provider_state).map_err(emit_boundary)?;
    if let Err(error) = verify_snapshot(&snapshot) {
        let _ = fs::remove_file(&output_path);
        return Err(emit_preflight(error));
    }
    println!("{}", serde_json::to_string_pretty(&receipt).map_err(|_| 2)?);
    Ok(())
}

fn run_probe(audio_path: &Path, provider_path: &Path, probe_path: &Path) -> Result<(), i32> {
    let snapshot = repo_snapshot()?;
    let audio_path = validated_input_path(audio_path, &snapshot.root).map_err(emit_boundary)?;
    let provider_path =
        validated_output_path(provider_path, &snapshot.root).map_err(emit_boundary)?;
    let probe_path = validated_output_path(probe_path, &snapshot.root).map_err(emit_boundary)?;
    if provider_path == probe_path {
        return Err(emit_boundary(BoundaryError::OutputPathsConflict));
    }
    let clean = worktree_clean()?;
    let prepared =
        prepare(&snapshot.candidate, clean, egress_authorized()).map_err(emit_preflight)?;
    let provider_state =
        serde_json::to_vec_pretty(&prepared.receipt().provider_state).map_err(|_| 2)?;
    let audio = fs::read(audio_path).map_err(|_| emit_boundary(BoundaryError::InputReadFailed))?;
    let probe = run_provider_probe(prepared, audio).map_err(|error| emit_probe(&error))?;
    let probe_bytes = serde_json::to_vec_pretty(&probe).map_err(|_| 2)?;

    verify_snapshot(&snapshot).map_err(emit_preflight)?;
    if let Err(error) = atomic_write(&provider_path, &provider_state) {
        return Err(emit_boundary(error));
    }
    if let Err(error) = atomic_write(&probe_path, &probe_bytes) {
        let _ = fs::remove_file(&provider_path);
        return Err(emit_boundary(error));
    }
    if let Err(error) = verify_snapshot(&snapshot) {
        let _ = fs::remove_file(&provider_path);
        let _ = fs::remove_file(&probe_path);
        return Err(emit_preflight(error));
    }
    println!("{}", serde_json::to_string_pretty(&probe).map_err(|_| 2)?);
    Ok(())
}

fn run_conversation(
    profile_path: &Path,
    owner_audio_path: &Path,
    visitor_audio_path: &Path,
    provider_path: &Path,
    receipt_path: &Path,
) -> Result<(), i32> {
    let snapshot = repo_snapshot()?;
    let profile_path = validated_input_path(profile_path, &snapshot.root).map_err(emit_boundary)?;
    let owner_audio_path =
        validated_input_path(owner_audio_path, &snapshot.root).map_err(emit_boundary)?;
    let visitor_audio_path =
        validated_input_path(visitor_audio_path, &snapshot.root).map_err(emit_boundary)?;
    let provider_path =
        validated_output_path(provider_path, &snapshot.root).map_err(emit_boundary)?;
    let receipt_path =
        validated_output_path(receipt_path, &snapshot.root).map_err(emit_boundary)?;
    ensure_unique_paths(&[
        profile_path.as_path(),
        owner_audio_path.as_path(),
        visitor_audio_path.as_path(),
        provider_path.as_path(),
        receipt_path.as_path(),
    ])
    .map_err(emit_boundary)?;

    let clean = worktree_clean()?;
    let prepared =
        prepare(&snapshot.candidate, clean, egress_authorized()).map_err(emit_preflight)?;
    let provider_state =
        serde_json::to_vec_pretty(&prepared.receipt().provider_state).map_err(|_| 2)?;
    let profile =
        fs::read(profile_path).map_err(|_| emit_boundary(BoundaryError::InputReadFailed))?;
    let owner_audio =
        fs::read(owner_audio_path).map_err(|_| emit_boundary(BoundaryError::InputReadFailed))?;
    let visitor_audio =
        fs::read(visitor_audio_path).map_err(|_| emit_boundary(BoundaryError::InputReadFailed))?;
    let receipt = run_live_conversation_attempt(prepared, &profile, owner_audio, visitor_audio)
        .map_err(emit_conversation)?;
    let receipt_bytes = serde_json::to_vec_pretty(&receipt).map_err(|_| 2)?;

    verify_snapshot(&snapshot).map_err(emit_preflight)?;
    if let Err(error) = atomic_write(&provider_path, &provider_state) {
        return Err(emit_boundary(error));
    }
    if let Err(error) = atomic_write(&receipt_path, &receipt_bytes) {
        let _ = fs::remove_file(&provider_path);
        return Err(emit_boundary(error));
    }
    if let Err(error) = verify_snapshot(&snapshot) {
        let _ = fs::remove_file(&provider_path);
        let _ = fs::remove_file(&receipt_path);
        return Err(emit_preflight(error));
    }
    println!("{}", serde_json::to_string_pretty(&receipt).map_err(|_| 2)?);
    Ok(())
}

fn repo_snapshot() -> Result<RepoSnapshot, i32> {
    Ok(RepoSnapshot {
        candidate: git_output(&["rev-parse", "HEAD"])?.trim().to_owned(),
        root: PathBuf::from(git_output(&["rev-parse", "--show-toplevel"])?.trim()),
    })
}

fn verify_snapshot(snapshot: &RepoSnapshot) -> Result<(), LiveProofPreflightError> {
    let current = git_output(&["rev-parse", "HEAD"])
        .map_err(|_| LiveProofPreflightError::CandidateChanged)?;
    if current.trim() != snapshot.candidate {
        return Err(LiveProofPreflightError::CandidateChanged);
    }
    if !worktree_clean().map_err(|_| LiveProofPreflightError::WorktreeDirty)? {
        return Err(LiveProofPreflightError::WorktreeDirty);
    }
    Ok(())
}

fn worktree_clean() -> Result<bool, i32> {
    Ok(
        git_output(&["status", "--porcelain", "--untracked-files=all"])?
            .trim()
            .is_empty(),
    )
}

fn egress_authorized() -> bool {
    env::var("VPR_LIVE_PROOF_ALLOW_EGRESS").is_ok_and(|value| value == "true")
}

fn validated_input_path(path: &Path, worktree_root: &Path) -> Result<PathBuf, BoundaryError> {
    if !path.is_absolute() {
        return Err(BoundaryError::InputPathInvalid);
    }
    let root = fs::canonicalize(worktree_root).map_err(|_| BoundaryError::InputPathInvalid)?;
    let resolved = fs::canonicalize(path).map_err(|_| BoundaryError::InputPathInvalid)?;
    if !resolved.is_file() {
        return Err(BoundaryError::InputPathInvalid);
    }
    if resolved.starts_with(root) {
        return Err(BoundaryError::InputPathInsideWorktree);
    }
    Ok(resolved)
}

fn validated_output_path(path: &Path, worktree_root: &Path) -> Result<PathBuf, BoundaryError> {
    if !path.is_absolute() {
        return Err(BoundaryError::OutputPathInvalid);
    }
    let root = fs::canonicalize(worktree_root).map_err(|_| BoundaryError::OutputPathInvalid)?;
    let parent = path.parent().ok_or(BoundaryError::OutputPathInvalid)?;
    let parent = fs::canonicalize(parent).map_err(|_| BoundaryError::OutputPathInvalid)?;
    let file_name = path.file_name().ok_or(BoundaryError::OutputPathInvalid)?;
    let resolved = parent.join(file_name);
    if resolved.starts_with(root) {
        return Err(BoundaryError::OutputPathInsideWorktree);
    }
    Ok(resolved)
}

fn ensure_unique_paths(paths: &[&Path]) -> Result<(), BoundaryError> {
    for (index, left) in paths.iter().enumerate() {
        if paths[index + 1..].iter().any(|right| left == right) {
            return Err(BoundaryError::OutputPathsConflict);
        }
    }
    Ok(())
}

fn git_output(args: &[&str]) -> Result<String, i32> {
    let output = Command::new("git").args(args).output().map_err(|_| 2)?;
    if !output.status.success() {
        return Err(2);
    }
    String::from_utf8(output.stdout).map_err(|_| 2)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), BoundaryError> {
    let parent = path.parent().ok_or(BoundaryError::ArtifactWriteFailed)?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(BoundaryError::ArtifactWriteFailed)?;
    let temp = parent.join(format!(".{file_name}.tmp-{}", std::process::id()));
    fs::write(&temp, bytes).map_err(|_| BoundaryError::ArtifactWriteFailed)?;
    fs::rename(&temp, path).map_err(|_| {
        let _ = fs::remove_file(&temp);
        BoundaryError::ArtifactWriteFailed
    })
}

fn emit_preflight(error: LiveProofPreflightError) -> i32 {
    let code = serde_json::to_value(error)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "INTERNAL_ERROR".into());
    emit_error(&code, None);
    2
}

fn emit_probe(error: &LiveProviderProbeError) -> i32 {
    emit_error(error.code(), Some(error.stage()));
    2
}

fn emit_conversation(error: LiveConversationAttemptError) -> i32 {
    emit_error(error.code(), Some(error.stage()));
    2
}

fn emit_boundary(error: BoundaryError) -> i32 {
    emit_error(error.code(), None);
    2
}

fn emit_error(code: &str, stage: Option<&str>) {
    eprintln!(
        "{}",
        serde_json::to_string(&CliError {
            ok: false,
            code,
            stage,
        })
        .unwrap_or_else(|_| "{\"ok\":false}".into())
    );
}
