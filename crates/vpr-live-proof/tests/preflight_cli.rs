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
fn untracked_files_cannot_be_hidden_by_git_config() {
    let repo = TempRepo::new();
    git(repo.path(), &["config", "status.showUntrackedFiles", "no"]);
    fs::write(
        repo.path().join("hidden-untracked.txt"),
        b"must still be dirty\n",
    )
    .unwrap();
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

fn probe_command(repo: &TempRepo, audio: &Path, provider: &Path, probe: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_vpr-live-proof"));
    command
        .current_dir(repo.path())
        .args(["probe"])
        .arg(audio)
        .arg(provider)
        .arg(probe)
        .env_clear();
    for key in ["PATH", "HOME", "USERPROFILE", "SYSTEMROOT"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
}

#[test]
fn probe_mode_fails_closed_without_egress_or_credentials_and_writes_nothing() {
    let repo = TempRepo::new();
    let audio = external_output(&repo, "probe-input.raw");
    let provider = external_output(&repo, "probe-provider.json");
    let probe = external_output(&repo, "probe-result.json");
    fs::write(&audio, vec![0_u8; 3_200]).unwrap();
    let output = probe_command(&repo, &audio, &provider, &probe)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("EGRESS_NOT_AUTHORIZED")
    );
    assert!(!provider.exists());
    assert!(!probe.exists());
    let _ = fs::remove_file(audio);
}

#[test]
fn probe_mode_rejects_private_audio_inside_candidate_checkout() {
    let repo = TempRepo::new();
    let audio = repo.path().join("private.raw");
    fs::write(&audio, vec![0_u8; 3_200]).unwrap();
    let provider = external_output(&repo, "probe-provider.json");
    let probe = external_output(&repo, "probe-result.json");
    let output = probe_command(&repo, &audio, &provider, &probe)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("INPUT_PATH_INSIDE_WORKTREE")
    );
    assert!(!provider.exists());
    assert!(!probe.exists());
}

fn conversation_command(
    repo: &TempRepo,
    profile: &Path,
    owner_audio: &Path,
    visitor_audio: &Path,
    provider: &Path,
    receipt: &Path,
) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_vpr-live-proof"));
    command
        .current_dir(repo.path())
        .arg("conversation")
        .arg(profile)
        .arg(owner_audio)
        .arg(visitor_audio)
        .arg(provider)
        .arg(receipt)
        .env_clear();
    for key in ["PATH", "HOME", "USERPROFILE", "SYSTEMROOT"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
}

fn write_conversation_inputs(repo: &TempRepo) -> (PathBuf, PathBuf, PathBuf) {
    let profile = external_output(repo, "conversation-profile.json");
    let owner_audio = external_output(repo, "conversation-owner.raw");
    let visitor_audio = external_output(repo, "conversation-visitor.raw");
    fs::write(
        &profile,
        br#"{"schema_version":"rt0-live-conversation-profile-0.1","persona_id":"p","owner_review_confirmed":true,"claims":[{"claim_id":"c","statement":"owner private material","kind":"opinion","owner_approved":true}]}"#,
    )
    .unwrap();
    fs::write(&owner_audio, vec![1_u8; 3_200]).unwrap();
    fs::write(&visitor_audio, vec![2_u8; 3_200]).unwrap();
    (profile, owner_audio, visitor_audio)
}

fn remove_inputs(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

#[test]
fn conversation_mode_rejects_private_profile_inside_candidate_checkout() {
    let repo = TempRepo::new();
    let profile = repo.path().join("private-profile.json");
    fs::write(&profile, b"{}\n").unwrap();
    let owner_audio = external_output(&repo, "owner.raw");
    let visitor_audio = external_output(&repo, "visitor.raw");
    let provider = external_output(&repo, "conversation-provider.json");
    let receipt = external_output(&repo, "conversation-receipt.json");
    fs::write(&owner_audio, vec![1_u8; 3_200]).unwrap();
    fs::write(&visitor_audio, vec![2_u8; 3_200]).unwrap();

    let output = conversation_command(
        &repo,
        &profile,
        &owner_audio,
        &visitor_audio,
        &provider,
        &receipt,
    )
    .output()
    .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("INPUT_PATH_INSIDE_WORKTREE")
    );
    assert!(!provider.exists());
    assert!(!receipt.exists());
    remove_inputs(&[owner_audio, visitor_audio]);
}

#[test]
fn conversation_mode_rejects_output_path_conflicts_before_egress() {
    let repo = TempRepo::new();
    let (profile, owner_audio, visitor_audio) = write_conversation_inputs(&repo);
    let output_path = external_output(&repo, "conversation-conflict.json");
    let output = conversation_command(
        &repo,
        &profile,
        &owner_audio,
        &visitor_audio,
        &output_path,
        &output_path,
    )
    .output()
    .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("OUTPUT_PATHS_CONFLICT")
    );
    assert!(!output_path.exists());
    remove_inputs(&[profile, owner_audio, visitor_audio]);
}

#[test]
fn conversation_mode_rejects_malformed_profile_without_provider_calls_or_writes() {
    let repo = TempRepo::new();
    let (profile, owner_audio, visitor_audio) = write_conversation_inputs(&repo);
    fs::write(&profile, b"not-json").unwrap();
    let provider = external_output(&repo, "conversation-provider.json");
    let receipt = external_output(&repo, "conversation-receipt.json");
    let mut command = conversation_command(
        &repo,
        &profile,
        &owner_audio,
        &visitor_audio,
        &provider,
        &receipt,
    );
    configure_live(&mut command, "did-secret", "stt-secret", "llm-secret");
    let output = command.output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("INVALID_PROFILE"));
    for secret in ["did-secret", "stt-secret", "llm-secret"] {
        assert!(!stderr.contains(secret));
    }
    assert!(!provider.exists());
    assert!(!receipt.exists());
    remove_inputs(&[profile, owner_audio, visitor_audio]);
}


fn candidate_command(
    repo: &TempRepo,
    probe_audio: &Path,
    profile: &Path,
    owner_audio: &Path,
    visitor_audio: &Path,
    provider: &Path,
    probe: &Path,
    receipt: &Path,
) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_vpr-live-proof"));
    command
        .current_dir(repo.path())
        .arg("candidate")
        .arg(probe_audio)
        .arg(profile)
        .arg(owner_audio)
        .arg(visitor_audio)
        .arg(provider)
        .arg(probe)
        .arg(receipt)
        .env_clear();
    for key in ["PATH", "HOME", "USERPROFILE", "SYSTEMROOT"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
}

#[test]
fn candidate_mode_rejects_invalid_probe_audio_before_egress_and_writes_nothing() {
    let repo = TempRepo::new();
    let probe_audio = external_output(&repo, "candidate-probe.raw");
    fs::write(&probe_audio, vec![0_u8; 3]).unwrap();
    let (profile, owner_audio, visitor_audio) = write_conversation_inputs(&repo);
    let provider = external_output(&repo, "candidate-provider.json");
    let probe = external_output(&repo, "candidate-probe.json");
    let receipt = external_output(&repo, "candidate-conversation.json");

    let output = candidate_command(
        &repo,
        &probe_audio,
        &profile,
        &owner_audio,
        &visitor_audio,
        &provider,
        &probe,
        &receipt,
    )
    .output()
    .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("INVALID_INPUT"));
    assert!(!stderr.contains("EGRESS_NOT_AUTHORIZED"));
    assert!(!provider.exists());
    assert!(!probe.exists());
    assert!(!receipt.exists());
    remove_inputs(&[probe_audio, profile, owner_audio, visitor_audio]);
}

#[test]
fn candidate_mode_rejects_malformed_profile_before_egress_and_writes_nothing() {
    let repo = TempRepo::new();
    let probe_audio = external_output(&repo, "candidate-probe.raw");
    fs::write(&probe_audio, vec![0_u8; 3_200]).unwrap();
    let (profile, owner_audio, visitor_audio) = write_conversation_inputs(&repo);
    fs::write(&profile, b"not-json").unwrap();
    let provider = external_output(&repo, "candidate-provider.json");
    let probe = external_output(&repo, "candidate-probe.json");
    let receipt = external_output(&repo, "candidate-conversation.json");

    let output = candidate_command(
        &repo,
        &probe_audio,
        &profile,
        &owner_audio,
        &visitor_audio,
        &provider,
        &probe,
        &receipt,
    )
    .output()
    .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("INVALID_PROFILE"));
    assert!(!stderr.contains("EGRESS_NOT_AUTHORIZED"));
    assert!(!provider.exists());
    assert!(!probe.exists());
    assert!(!receipt.exists());
    remove_inputs(&[probe_audio, profile, owner_audio, visitor_audio]);
}

#[test]
fn candidate_mode_rejects_output_conflicts_before_egress_and_writes_nothing() {
    let repo = TempRepo::new();
    let probe_audio = external_output(&repo, "candidate-probe.raw");
    fs::write(&probe_audio, vec![0_u8; 3_200]).unwrap();
    let (profile, owner_audio, visitor_audio) = write_conversation_inputs(&repo);
    let shared_output = external_output(&repo, "candidate-conflict.json");
    let receipt = external_output(&repo, "candidate-conversation.json");

    let output = candidate_command(
        &repo,
        &probe_audio,
        &profile,
        &owner_audio,
        &visitor_audio,
        &shared_output,
        &shared_output,
        &receipt,
    )
    .output()
    .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("OUTPUT_PATHS_CONFLICT"));
    assert!(!stderr.contains("EGRESS_NOT_AUTHORIZED"));
    assert!(!shared_output.exists());
    assert!(!receipt.exists());
    remove_inputs(&[probe_audio, profile, owner_audio, visitor_audio]);
}
