use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use vpr_evaluation::{RT0_RELEASE_SPEC_BYTES, sha256_hex};

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
            "vpr-evidence-prepare-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn child(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(path: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_vpr-rt0-evidence-prepare"))
        .arg(path)
        .output()
        .unwrap()
}

#[test]
fn prepare_materializes_only_exact_release_spec_and_reports_real_gaps() {
    let temp = TempDir::new();
    let evidence = temp.child("rt0");
    let output = run(&evidence);
    assert_eq!(output.status.code(), Some(0));

    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema_version"], json!("rt0-evidence-preparation-0.1"));
    assert_eq!(report["workspace_prepared"], json!(true));
    assert_eq!(report["release_ready_claimed"], json!(false));
    assert_eq!(report["release_spec_written"], json!(true));
    assert_eq!(
        report["release_spec_sha256"],
        json!(sha256_hex(RT0_RELEASE_SPEC_BYTES))
    );
    assert_eq!(
        fs::read(evidence.join("release-spec.md")).unwrap(),
        RT0_RELEASE_SPEC_BYTES
    );

    for forbidden_placeholder in [
        "provider-state.json",
        "private-golden-evidence.json",
        "human-evaluation.json",
        "privacy-permissions.json",
        "exit-evidence.json",
    ] {
        assert!(!evidence.join(forbidden_placeholder).exists());
        assert!(
            report["missing_artifacts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == forbidden_placeholder)
        );
    }
}

#[test]
fn prepare_is_idempotent_for_exact_release_spec() {
    let temp = TempDir::new();
    let evidence = temp.child("rt0");
    assert_eq!(run(&evidence).status.code(), Some(0));
    let second = run(&evidence);
    assert_eq!(second.status.code(), Some(0));
    let report: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(report["release_spec_written"], json!(false));
    assert_eq!(
        fs::read(evidence.join("release-spec.md")).unwrap(),
        RT0_RELEASE_SPEC_BYTES
    );
}

#[test]
fn prepare_refuses_to_overwrite_conflicting_release_spec() {
    let temp = TempDir::new();
    let evidence = temp.child("rt0");
    fs::create_dir(&evidence).unwrap();
    fs::write(evidence.join("release-spec.md"), b"stale release spec").unwrap();

    let output = run(&evidence);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("RELEASE_SPEC_CONFLICT"));
    assert_eq!(
        fs::read(evidence.join("release-spec.md")).unwrap(),
        b"stale release spec"
    );
}
