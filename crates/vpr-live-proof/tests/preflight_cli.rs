use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use vpr_evaluation::ProviderStateManifest;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempRepo(PathBuf);

impl TempRepo {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "vpr-live-proof-{}-{nanos}-{seq}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        git(&path, &["init", "-q"]);
        git(&path, &["config", "user.email", "proof@example.invalid"]);
        git(&path, &["config", "user.name", "Proof Test"]);
        fs::write(path.join("tracked.txt"), b"candidate\n").unwrap();
        git(&path, &["add", "tracked.txt"]);
        git(&path, &["commit", "-qm", "candidate"]);
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(repo)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success());
}

fn external_output(repo: &TempRepo, suffix: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "{}-{suffix}",
        repo.path().file_name().unwrap().to_string_lossy()
    ))
}

fn command(repo: &TempRepo, output: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_vpr-live-proof"));
    command.current_dir(repo.path()).arg(output).env_clear();
    for key in ["PATH", "HOME", "USERPROFILE", "SYSTEMROOT"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
}

fn configure_live(command: &mut Command, did_key: &str, stt_key: &str, llm_key: &str) {
    command
        .env("VPR_LIVE_PROOF_ALLOW_EGRESS", "true")
        .env("VPR_DID_ENDPOINT", "https://api.d-id.com")
        .env("VPR_DID_API_KEY", did_key)
        .env("VPR_DID_AGENT_ID", "agent-contract")
        .env("VPR_OWNER_LAB_STT_PROVIDER", "openai-transcription")
        .env(
            "VPR_OWNER_LAB_STT_ENDPOINT",
            "https://api.openai.com/v1/audio/transcriptions",
        )
        .env("VPR_OWNER_LAB_STT_API_KEY", stt_key)
        .env("VPR_OWNER_LAB_STT_MODEL", "whisper-contract")
        .env("VPR_OWNER_LAB_LLM_PROVIDER", "openai-compatible")
        .env(
            "VPR_OWNER_LAB_LLM_ENDPOINT",
            "https://api.openai.com/v1/chat/completions",
        )
        .env("VPR_OWNER_LAB_LLM_API_KEY", llm_key)
        .env("VPR_OWNER_LAB_LLM_MODEL", "gpt-contract");
}

#[test]
fn missing_egress_authorization_fails_before_credentials() {
    let repo = TempRepo::new();
    let output_path = external_output(&repo, "providers.json");
    let output = command(&repo, &output_path).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("EGRESS_NOT_AUTHORIZED")
    );
    assert!(!output_path.exists());
}

#[test]
fn dirty_worktree_fails_before_provider_construction() {
    let repo = TempRepo::new();
    fs::write(repo.path().join("tracked.txt"), b"dirty\n").unwrap();
    let output_path = external_output(&repo, "providers.json");
    let output = command(&repo, &output_path)
        .env("VPR_LIVE_PROOF_ALLOW_EGRESS", "true")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("WORKTREE_DIRTY")
    );
    assert!(!output_path.exists());
}

#[test]
fn incomplete_credentials_fail_closed_without_secret_echo() {
    let repo = TempRepo::new();
    let output_path = external_output(&repo, "providers.json");
    let output = command(&repo, &output_path)
        .env("VPR_LIVE_PROOF_ALLOW_EGRESS", "true")
        .env("VPR_DID_API_KEY", "do-not-echo-me")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("PROVIDER_CONFIGURATION_INVALID"));
    assert!(!stderr.contains("do-not-echo-me"));
    assert!(!output_path.exists());
}

#[test]
fn successful_preflight_is_sanitized_and_secret_independent() {
    let repo = TempRepo::new();
    let first_path = external_output(&repo, "providers-1.json");
    let mut first = command(&repo, &first_path);
    configure_live(
        &mut first,
        "did-secret-one",
        "stt-secret-one",
        "llm-secret-one",
    );
    let first = first.output().unwrap();
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );

    let second_path = external_output(&repo, "providers-2.json");
    let mut second = command(&repo, &second_path);
    configure_live(
        &mut second,
        "did-secret-two",
        "stt-secret-two",
        "llm-secret-two",
    );
    let second = second.output().unwrap();
    assert_eq!(
        second.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );

    let first_state = fs::read(&first_path).unwrap();
    let second_state = fs::read(&second_path).unwrap();
    assert_eq!(
        first_state, second_state,
        "credential rotation must not change sanitized provider state"
    );
    let parsed: ProviderStateManifest = serde_json::from_slice(&first_state).unwrap();
    assert_eq!(parsed.providers.len(), 3);

    let receipt: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(receipt["schema_version"], "rt0-live-proof-preflight-0.1");
    assert_eq!(
        receipt["provider_state_sha256"],
        vpr_evaluation::sha256_hex(&first_state)
    );
    let all_output = format!(
        "{}{}{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
        String::from_utf8_lossy(&first_state)
    );
    for secret in ["did-secret-one", "stt-secret-one", "llm-secret-one"] {
        assert!(!all_output.contains(secret));
    }
    let _ = fs::remove_file(first_path);
    let _ = fs::remove_file(second_path);
}

#[test]
fn output_path_must_be_absolute_and_outside_worktree() {
    let repo = TempRepo::new();

    let relative = PathBuf::from("providers.json");
    let output = command(&repo, &relative).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("OUTPUT_PATH_INVALID")
    );

    let inside = repo.path().join("providers.json");
    let output = command(&repo, &inside).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("OUTPUT_PATH_INSIDE_WORKTREE")
    );
    assert!(!inside.exists());
}
