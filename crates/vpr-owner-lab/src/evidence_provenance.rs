use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

use vpr_evaluation::{sha256_hex, validate_candidate_sha};
use vpr_owner_lab::ProviderBundle;

pub fn resolve(
    providers: &ProviderBundle,
    strict: bool,
) -> Result<Option<(String, String)>, Box<dyn Error + Send + Sync>> {
    let candidate = match current_clean_candidate() {
        Ok(candidate) => candidate,
        Err(error) if !strict => {
            eprintln!("Owner Lab evidence remains non-release-bound: {error}");
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let provider_state = match providers.provider_state_manifest() {
        Ok(provider_state) => provider_state,
        Err(error) if !strict => {
            eprintln!("Owner Lab evidence remains non-release-bound: {error}");
            return Ok(None);
        }
        Err(error) => return Err(error.into()),
    };
    let provider_state_bytes = serde_json::to_vec_pretty(&provider_state)?;
    if let Some(out) = env::var_os("VPR_OWNER_LAB_PROVIDER_STATE_OUT") {
        let worktree = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .output()?;
        if !worktree.status.success() {
            return Err("cannot locate worktree for provider-state evidence".into());
        }
        let root = PathBuf::from(String::from_utf8(worktree.stdout)?.trim());
        write_provider_state(Path::new(&out), &root, &provider_state_bytes)?;
    }
    Ok(Some((candidate, sha256_hex(&provider_state_bytes))))
}

/// Persist precisely the bytes bound by the session evidence digest.
/// The output must not contaminate the exact-candidate Git worktree.
fn write_provider_state(
    out: &Path,
    worktree: &Path,
    bytes: &[u8],
) -> Result<(), Box<dyn Error + Send + Sync>> {
    if !out.is_absolute() {
        return Err("VPR_OWNER_LAB_PROVIDER_STATE_OUT must be absolute".into());
    }
    let parent = out
        .parent()
        .ok_or("provider-state output has no parent")?
        .canonicalize()?;
    if parent.starts_with(worktree.canonicalize()?) {
        return Err("provider-state output must be outside the Git worktree".into());
    }
    match fs::read(out) {
        Ok(existing) if existing == bytes => return Ok(()),
        Ok(_) => return Err("PROVIDER_STATE_OUTPUT_CONFLICT: provider state changed".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let staging = out.with_extension("partial");
    fs::write(&staging, bytes)?;
    fs::rename(&staging, out)?;
    Ok(())
}

fn current_clean_candidate() -> Result<String, Box<dyn Error + Send + Sync>> {
    let head = Command::new("git").args(["rev-parse", "HEAD"]).output()?;
    if !head.status.success() {
        return Err("could not resolve exact Git candidate for Owner Lab evidence".into());
    }
    let candidate = String::from_utf8(head.stdout)?.trim().to_owned();
    validate_candidate_sha(&candidate)
        .map_err(|_| "Owner Lab evidence candidate SHA is invalid")?;

    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .output()?;
    if !status.status.success() {
        return Err("could not inspect Git worktree for Owner Lab evidence".into());
    }
    if !status.stdout.is_empty() {
        return Err("Owner Lab RT0 evidence requires a clean Git worktree".into());
    }
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::write_provider_state;
    use std::fs;

    #[test]
    fn provider_state_is_published_exactly_once_outside_worktree() {
        let base = std::env::temp_dir().join(format!("vpr-ps-{}", std::process::id()));
        let worktree = base.join("repo");
        let evidence = base.join("evidence");
        fs::create_dir_all(&worktree).unwrap();
        fs::create_dir_all(&evidence).unwrap();
        let out = evidence.join("provider-state.json");
        write_provider_state(&out, &worktree, b"{\"a\":1}").unwrap();
        assert_eq!(fs::read(&out).unwrap(), b"{\"a\":1}");
        write_provider_state(&out, &worktree, b"{\"a\":1}").unwrap();
        assert!(write_provider_state(&out, &worktree, b"{\"a\":2}").is_err());
        assert!(write_provider_state(&worktree.join("p.json"), &worktree, b"{}").is_err());
        assert!(write_provider_state(std::path::Path::new("rel.json"), &worktree, b"{}").is_err());
        fs::remove_dir_all(&base).unwrap();
    }
}
