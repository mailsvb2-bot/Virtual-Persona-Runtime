use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::{env, process};

use serde::Serialize;
use vpr_evaluation::{
    Rt0ExitAssemblyInputs, Rt0SupportingPreflightArtifacts, assemble_rt0_exit_evidence, sha256_hex,
};

#[derive(Serialize)]
struct CliError<T: Serialize> {
    ok: bool,
    code: T,
}

#[derive(Serialize)]
struct AssemblyReceipt<'a> {
    ok: bool,
    candidate_sha: &'a str,
    output_sha256: String,
}

struct SupportingArtifactBytes {
    ci: Vec<u8>,
    e2e: Vec<u8>,
    owner_conversation: Vec<u8>,
    visitor_conversation: Vec<u8>,
    acceptance: Vec<u8>,
    quality: Vec<u8>,
    cost: Vec<u8>,
    privacy_permissions: Vec<u8>,
    human_evaluation: Vec<u8>,
    known_limitations: Vec<u8>,
}

impl SupportingArtifactBytes {
    fn read(root: &Path) -> Result<Self, i32> {
        if !root.is_dir() {
            return fail("INPUT_INVALID");
        }
        Ok(Self {
            ci: read_path(&root.join("ci-evidence.json"))?,
            e2e: read_path(&root.join("e2e-evidence.json"))?,
            owner_conversation: read_path(&root.join("owner-conversation.json"))?,
            visitor_conversation: read_path(&root.join("visitor-conversation.json"))?,
            acceptance: read_path(&root.join("acceptance.json"))?,
            quality: read_path(&root.join("quality.json"))?,
            cost: read_path(&root.join("cost.json"))?,
            privacy_permissions: read_path(&root.join("privacy-permissions.json"))?,
            human_evaluation: read_path(&root.join("human-evaluation.json"))?,
            known_limitations: read_path(&root.join("known-limitations.md"))?,
        })
    }

    fn as_preflight(&self) -> Rt0SupportingPreflightArtifacts<'_> {
        Rt0SupportingPreflightArtifacts {
            ci: &self.ci,
            e2e: &self.e2e,
            owner_conversation: &self.owner_conversation,
            visitor_conversation: &self.visitor_conversation,
            acceptance: &self.acceptance,
            quality: &self.quality,
            cost: &self.cost,
            privacy_permissions: &self.privacy_permissions,
            human_evaluation: &self.human_evaluation,
            known_limitations: &self.known_limitations,
        }
    }
}

fn main() {
    if let Err(code) = run() {
        process::exit(code);
    }
}

fn run() -> Result<(), i32> {
    let args: Vec<String> = env::args().skip(1).collect();
    let [
        supporting_dir,
        golden_report_path,
        provider_state_path,
        provider_probe_path,
        conversation_attempt_path,
        bound_session_aggregate_path,
        release_spec_path,
        output_path,
        candidate_sha,
    ] = args.as_slice()
    else {
        eprintln!(
            "usage: vpr-rt0-exit-assemble <supporting-evidence-dir> <bound-golden-report.json> <provider-state.json> <live-provider-probe.json> <conversation-attempt.json> <bound-session-aggregate.json> <release-spec.md> <output-exit-evidence.json> <exact-candidate-sha>"
        );
        return Err(2);
    };

    let output_path = Path::new(output_path);
    if output_path.exists() {
        return fail("OUTPUT_EXISTS");
    }
    let supporting = SupportingArtifactBytes::read(Path::new(supporting_dir))?;
    let golden_report = read_path(Path::new(golden_report_path))?;
    let provider_state = read_path(Path::new(provider_state_path))?;
    let provider_probe = read_path(Path::new(provider_probe_path))?;
    let conversation_attempt = read_path(Path::new(conversation_attempt_path))?;
    let bound_session_aggregate = read_path(Path::new(bound_session_aggregate_path))?;
    let release_spec = read_path(Path::new(release_spec_path))?;

    let evidence = assemble_rt0_exit_evidence(Rt0ExitAssemblyInputs {
        supporting: supporting.as_preflight(),
        bound_golden_report_bytes: &golden_report,
        provider_state_bytes: &provider_state,
        live_provider_probe_bytes: &provider_probe,
        conversation_attempt_bytes: &conversation_attempt,
        bound_session_aggregate_bytes: &bound_session_aggregate,
        release_spec_bytes: &release_spec,
        exact_candidate_sha: candidate_sha,
    })
    .map_err(|code| {
        let _ = emit_error(code);
        2
    })?;

    let output_bytes = serde_json::to_vec_pretty(&evidence).map_err(|_| 2)?;
    write_new_atomic(output_path, &output_bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&AssemblyReceipt {
            ok: true,
            candidate_sha,
            output_sha256: sha256_hex(&output_bytes),
        })
        .map_err(|_| 2)?
    );
    Ok(())
}

fn read_path(path: &Path) -> Result<Vec<u8>, i32> {
    fs::read(path).map_err(|_| {
        let _ = emit_error("INPUT_INVALID");
        2
    })
}

fn write_new_atomic(path: &Path, bytes: &[u8]) -> Result<(), i32> {
    if path.exists() {
        return fail("OUTPUT_EXISTS");
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return fail("OUTPUT_PATH_INVALID");
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| emit_code("OUTPUT_PATH_INVALID"))?;
    let temp_path: PathBuf = parent.join(format!(".{file_name}.tmp"));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp_path)
        .map_err(|_| emit_code("OUTPUT_TEMP_UNAVAILABLE"))?;
    if file.write_all(bytes).is_err() || file.sync_all().is_err() {
        drop(file);
        let _ = fs::remove_file(&temp_path);
        return fail("OUTPUT_WRITE_FAILED");
    }
    drop(file);
    if fs::rename(&temp_path, path).is_err() {
        let _ = fs::remove_file(&temp_path);
        return fail("OUTPUT_COMMIT_FAILED");
    }
    Ok(())
}

fn emit_code(code: &'static str) -> i32 {
    let _ = emit_error(code);
    2
}

fn fail<T>(code: &'static str) -> Result<T, i32> {
    Err(emit_code(code))
}

fn emit_error<T: Serialize>(code: T) -> Result<(), i32> {
    eprintln!(
        "{}",
        serde_json::to_string(&CliError { ok: false, code }).map_err(|_| 2)?
    );
    Ok(())
}


#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::write_new_atomic;

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "vpr-exit-assemble-{}-{nanos}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn atomic_writer_commits_exact_bytes_without_temp_artifact() {
        let dir = TempDir::new();
        let output = dir.0.join("exit-evidence.json");
        write_new_atomic(&output, b"exact manifest bytes").unwrap();

        assert_eq!(fs::read(&output).unwrap(), b"exact manifest bytes");
        assert!(!dir.0.join(".exit-evidence.json.tmp").exists());
    }

    #[test]
    fn atomic_writer_never_overwrites_existing_manifest() {
        let dir = TempDir::new();
        let output = dir.0.join("exit-evidence.json");
        fs::write(&output, b"existing").unwrap();

        assert_eq!(write_new_atomic(&output, b"replacement"), Err(2));
        assert_eq!(fs::read(&output).unwrap(), b"existing");
        assert!(!dir.0.join(".exit-evidence.json.tmp").exists());
    }
}
