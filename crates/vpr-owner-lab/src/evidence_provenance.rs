use std::error::Error;
use std::process::Command;

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
    Ok(Some((candidate, sha256_hex(&provider_state_bytes))))
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
