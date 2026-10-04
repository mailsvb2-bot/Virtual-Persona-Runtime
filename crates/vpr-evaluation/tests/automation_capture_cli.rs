use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::{Value, json};

const CANDIDATE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "vpr-automation-capture-cli-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn scaffold(root: &TempDir) {
    let supporting = root.0.join("supporting");
    fs::create_dir_all(&supporting).unwrap();
    for name in ["ci-evidence.json", "e2e-evidence.json"] {
        fs::write(
            supporting.join(name),
            serde_json::to_vec_pretty(&json!({
                "status":"failed",
                "candidate_sha":CANDIDATE
            }))
            .unwrap()
            .into_iter()
            .chain([b'\n'])
            .collect::<Vec<_>>(),
        )
        .unwrap();
    }
}

fn reviewed_input(root: &TempDir, attestation: &str) -> PathBuf {
    let path = root.0.join("automation.json");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "schema_version":"rt0-automation-observations-0.1",
            "candidate_sha":CANDIDATE,
            "attestation":attestation,
            "ci":{
                "status":"passed",
                "evidence_reference":"github-actions:ci:37191364067"
            },
            "e2e":{
                "status":"passed",
                "evidence_reference":"github-actions:e2e:37191364067"
            }
        }))
        .unwrap(),
    )
    .unwrap();
    path
}

fn run(root: &TempDir, input: &PathBuf) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_vpr-rt0-automation-capture"))
        .arg(input)
        .arg(root.0.join("supporting"))
        .arg(CANDIDATE)
        .output()
        .unwrap()
}

#[test]
fn reviewed_automation_replaces_only_canonical_ci_e2e_scaffold() {
    let root = TempDir::new();
    scaffold(&root);
    let input = reviewed_input(&root, "reviewed_exact_candidate_automation");

    let output = run(&root, &input);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    for name in ["ci-evidence.json", "e2e-evidence.json"] {
        let value: Value =
            serde_json::from_slice(&fs::read(root.0.join("supporting").join(name)).unwrap()).unwrap();
        assert_eq!(value["status"], "passed");
        assert_eq!(value["candidate_sha"], CANDIDATE);
        assert!(value.get("evidence_reference").is_none());
    }

    let receipt: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        receipt["schema_version"],
        "rt0-automation-supporting-capture-receipt-0.1"
    );
    assert_eq!(receipt["already_committed"], false);

    let second = run(&root, &input);
    assert!(second.status.success());
    let receipt: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(receipt["already_committed"], true);
}

#[test]
fn review_attestation_is_required_before_any_scaffold_change() {
    let root = TempDir::new();
    scaffold(&root);
    let input = reviewed_input(&root, "REVIEW_REQUIRED");

    let output = run(&root, &input);
    assert_eq!(output.status.code(), Some(2));

    for name in ["ci-evidence.json", "e2e-evidence.json"] {
        let value: Value =
            serde_json::from_slice(&fs::read(root.0.join("supporting").join(name)).unwrap()).unwrap();
        assert_eq!(value["status"], "failed");
    }
}

#[test]
fn modified_automation_file_is_never_overwritten() {
    let root = TempDir::new();
    scaffold(&root);
    let input = reviewed_input(&root, "reviewed_exact_candidate_automation");
    fs::write(
        root.0.join("supporting").join("ci-evidence.json"),
        b"{\"status\":\"passed\",\"candidate_sha\":\"operator-owned\"}\n",
    )
    .unwrap();

    let output = run(&root, &input);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        fs::read(root.0.join("supporting").join("ci-evidence.json")).unwrap(),
        b"{\"status\":\"passed\",\"candidate_sha\":\"operator-owned\"}\n"
    );
}
