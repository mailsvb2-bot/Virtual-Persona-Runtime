use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use vpr_evaluation::{
    RT0_EVIDENCE_REQUIRED_FILES, RT0_RELEASE_SPEC_BYTES, prepare_rt0_evidence_workspace, sha256_hex,
};

#[derive(Serialize)]
struct CliError<'a> {
    ok: bool,
    code: &'a str,
}

#[derive(Serialize)]
struct PreparationReport {
    schema_version: &'static str,
    candidate_sha: String,
    evidence_dir: String,
    workspace_prepared: bool,
    release_ready_claimed: bool,
    release_spec_sha256: String,
    release_spec_written: bool,
    present_artifacts: Vec<&'static str>,
    missing_artifacts: Vec<&'static str>,
}

struct RepoSnapshot {
    candidate_sha: String,
    root: PathBuf,
}

fn main() {
    if let Err(code) = run() {
        std::process::exit(code);
    }
}

fn run() -> Result<(), i32> {
    let args: Vec<String> = env::args().skip(1).collect();
    let [evidence_dir] = args.as_slice() else {
        eprintln!("usage: vpr-rt0-evidence-prepare <absolute-evidence-dir>");
        return Err(2);
    };

    let snapshot = repo_snapshot()?;
    verify_snapshot(&snapshot)?;
    verify_embedded_release_spec(&snapshot.root)?;
    let evidence_dir = validated_workspace_path(Path::new(evidence_dir), &snapshot.root)?;

    let release_spec_written = prepare_rt0_evidence_workspace(&evidence_dir).map_err(|error| {
        emit_error(match error {
            vpr_evaluation::Rt0EvidenceWorkspaceError::CreateDirectoryFailed => {
                "EVIDENCE_DIRECTORY_CREATE_FAILED"
            }
            vpr_evaluation::Rt0EvidenceWorkspaceError::ReleaseSpecConflict => {
                "RELEASE_SPEC_CONFLICT"
            }
            vpr_evaluation::Rt0EvidenceWorkspaceError::ReleaseSpecReadFailed => {
                "RELEASE_SPEC_READ_FAILED"
            }
            vpr_evaluation::Rt0EvidenceWorkspaceError::ReleaseSpecWriteFailed => {
                "RELEASE_SPEC_WRITE_FAILED"
            }
        })
    })?;

    let resolved_evidence_dir =
        fs::canonicalize(&evidence_dir).map_err(|_| emit_error("EVIDENCE_PATH_INVALID"))?;
    let resolved_root =
        fs::canonicalize(&snapshot.root).map_err(|_| emit_error("WORKTREE_PATH_INVALID"))?;
    if resolved_evidence_dir.starts_with(&resolved_root) {
        if release_spec_written {
            let _ = fs::remove_file(resolved_evidence_dir.join("release-spec.md"));
        }
        return Err(emit_error("EVIDENCE_PATH_INSIDE_WORKTREE"));
    }

    if let Err(code) = verify_snapshot(&snapshot) {
        if release_spec_written {
            let _ = fs::remove_file(resolved_evidence_dir.join("release-spec.md"));
        }
        return Err(code);
    }

    let mut present_artifacts = Vec::new();
    let mut missing_artifacts = Vec::new();
    for &name in RT0_EVIDENCE_REQUIRED_FILES {
        if resolved_evidence_dir.join(name).is_file() {
            present_artifacts.push(name);
        } else {
            missing_artifacts.push(name);
        }
    }

    let report = PreparationReport {
        schema_version: "rt0-evidence-preparation-0.1",
        candidate_sha: snapshot.candidate_sha,
        evidence_dir: resolved_evidence_dir.to_string_lossy().into_owned(),
        workspace_prepared: true,
        release_ready_claimed: false,
        release_spec_sha256: sha256_hex(RT0_RELEASE_SPEC_BYTES),
        release_spec_written,
        present_artifacts,
        missing_artifacts,
    };
    println!("{}", serde_json::to_string_pretty(&report).map_err(|_| 2)?);
    Ok(())
}

fn repo_snapshot() -> Result<RepoSnapshot, i32> {
    let candidate_sha = git_output(&["rev-parse", "HEAD"])?;
    let root = git_output(&["rev-parse", "--show-toplevel"])?;
    Ok(RepoSnapshot {
        candidate_sha: candidate_sha.trim().to_owned(),
        root: PathBuf::from(root.trim()),
    })
}

fn verify_snapshot(snapshot: &RepoSnapshot) -> Result<(), i32> {
    let current = git_output(&["rev-parse", "HEAD"])?;
    if current.trim() != snapshot.candidate_sha {
        return Err(emit_error("CANDIDATE_CHANGED"));
    }
    let status = git_output(&["status", "--porcelain", "--untracked-files=all"])?;
    if !status.trim().is_empty() {
        return Err(emit_error("WORKTREE_DIRTY"));
    }
    Ok(())
}

fn verify_embedded_release_spec(worktree_root: &Path) -> Result<(), i32> {
    let bytes = fs::read(worktree_root.join("docs/releases/RT0_RELEASE_SPEC.md"))
        .map_err(|_| emit_error("RELEASE_SPEC_READ_FAILED"))?;
    if bytes != RT0_RELEASE_SPEC_BYTES {
        return Err(emit_error("STALE_BINARY_RELEASE_SPEC"));
    }
    Ok(())
}

fn validated_workspace_path(path: &Path, worktree_root: &Path) -> Result<PathBuf, i32> {
    if !path.is_absolute() {
        return Err(emit_error("EVIDENCE_PATH_INVALID"));
    }

    let root = fs::canonicalize(worktree_root).map_err(|_| emit_error("WORKTREE_PATH_INVALID"))?;
    let resolved = if path.exists() {
        let resolved = fs::canonicalize(path).map_err(|_| emit_error("EVIDENCE_PATH_INVALID"))?;
        if !resolved.is_dir() {
            return Err(emit_error("EVIDENCE_PATH_INVALID"));
        }
        resolved
    } else {
        if path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        }) {
            return Err(emit_error("EVIDENCE_PATH_INVALID"));
        }
        let ancestor = path
            .ancestors()
            .find(|candidate| candidate.exists())
            .ok_or_else(|| emit_error("EVIDENCE_PATH_INVALID"))?;
        if !ancestor.is_dir() {
            return Err(emit_error("EVIDENCE_PATH_INVALID"));
        }
        let suffix = path
            .strip_prefix(ancestor)
            .map_err(|_| emit_error("EVIDENCE_PATH_INVALID"))?;
        let ancestor =
            fs::canonicalize(ancestor).map_err(|_| emit_error("EVIDENCE_PATH_INVALID"))?;
        ancestor.join(suffix)
    };

    if resolved.starts_with(root) {
        return Err(emit_error("EVIDENCE_PATH_INSIDE_WORKTREE"));
    }
    Ok(resolved)
}

fn git_output(args: &[&str]) -> Result<String, i32> {
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|_| emit_error("GIT_UNAVAILABLE"))?;
    if !output.status.success() {
        return Err(emit_error("GIT_UNAVAILABLE"));
    }
    String::from_utf8(output.stdout).map_err(|_| emit_error("GIT_OUTPUT_INVALID"))
}

fn emit_error(code: &'static str) -> i32 {
    eprintln!(
        "{}",
        serde_json::to_string(&CliError { ok: false, code })
            .unwrap_or_else(|_| r#"{"ok":false}"#.into())
    );
    2
}
