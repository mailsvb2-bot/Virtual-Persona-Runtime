mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use vpr_evaluation::sha256_hex;

const CANDIDATE: &str = "1111111111111111111111111111111111111111";
const RELEASE_SPEC: &[u8] = b"rt0 inventory release spec";
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
            "vpr-inventory-cli-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(dir: &Path, candidate: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_vpr-rt0-evidence-inventory"))
        .arg(dir)
        .arg(candidate)
        .output()
        .unwrap()
}

#[test]
fn empty_directory_reports_missing_evidence_without_claiming_readiness() {
    let dir = TempDir::new();
    let output = run(dir.path(), CANDIDATE);
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["inventory_complete"], json!(false));
    assert_eq!(report["bindings"]["candidate_sha_valid"], json!(true));
    assert!(report["missing"].as_array().unwrap().len() >= 10);
    assert!(report.get("ready").is_none());
    assert!(
        report["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| !item["present"].as_bool().unwrap())
    );
}
fn seed_complete_inventory(dir: &Path) {
    let fixture = support::fixture(RELEASE_SPEC, CANDIDATE);
    assert_eq!(fixture.provider_state.providers.len(), 3);
    assert_eq!(fixture.bundle.binding.candidate_sha, CANDIDATE);
    let provider_digest = sha256_hex(&fixture.provider_state_bytes);
    fs::write(
        dir.join("provider-state.json"),
        &fixture.provider_state_bytes,
    )
    .unwrap();
    fs::write(
        dir.join("bound-golden-report.json"),
        serde_json::to_vec_pretty(&fixture.report).unwrap(),
    )
    .unwrap();
    fs::write(
        dir.join("private-golden-evidence.json"),
        fixture.bundle_bytes,
    )
    .unwrap();

    let usage = json!({
        "input_units":1,"input_unit":"token","output_units":1,"output_unit":"token",
        "estimated_cost_microunits":1,"provider_charge_microunits":null
    });
    let probe = json!({
        "schema_version":"rt0-live-provider-probe-0.1",
        "candidate_sha":CANDIDATE,
        "provider_state_sha256":provider_digest,
        "input_audio_sha256":"7".repeat(64),
        "input_audio_millis":1000,
        "scope":"credentialed_provider_reachability_only",
        "conversation_evidence":false,
        "output_delivery_proven":false,
        "stt":{"latency_millis":100,"transcript_chars":6,"usage":usage.clone()},
        "llm":{"latency_millis":120,"output_chars":5,"usage":usage},
        "avatar":{"open_millis":150,"close_millis":50}
    });
    fs::write(
        dir.join("provider-probe.json"),
        serde_json::to_vec_pretty(&probe).unwrap(),
    )
    .unwrap();
    for name in [
        "exit-evidence.json",
        "ci-evidence.json",
        "e2e-evidence.json",
        "owner-conversation.json",
        "visitor-conversation.json",
        "acceptance.json",
        "quality.json",
        "cost.json",
        "privacy-permissions.json",
        "human-evaluation.json",
        "known-limitations.md",
    ] {
        if Path::new(name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        {
            fs::write(dir.join(name), b"{}\n").unwrap();
        } else {
            fs::write(dir.join(name), format!("inventory fixture: {name}\n")).unwrap();
        }
    }
}

#[test]
fn complete_inventory_requires_exact_candidate_and_provider_binding() {
    let dir = TempDir::new();
    seed_complete_inventory(dir.path());
    let output = run(dir.path(), CANDIDATE);
    assert_eq!(output.status.code(), Some(0));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["inventory_complete"], json!(true));
    assert_eq!(report["missing"], json!([]));
    assert_eq!(report["bindings"]["golden_candidate_matches"], json!(true));
    assert_eq!(report["bindings"]["probe_candidate_matches"], json!(true));
    assert_eq!(
        report["bindings"]["golden_provider_state_matches"],
        json!(true)
    );
    assert_eq!(
        report["bindings"]["probe_provider_state_matches"],
        json!(true)
    );
}
#[test]
fn cross_candidate_inventory_fails_closed() {
    let dir = TempDir::new();
    seed_complete_inventory(dir.path());
    let output = run(dir.path(), &"2".repeat(40));
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["inventory_complete"], json!(false));
    assert_eq!(report["bindings"]["candidate_sha_valid"], json!(true));
    assert_eq!(report["bindings"]["golden_candidate_matches"], json!(false));
    assert_eq!(report["bindings"]["probe_candidate_matches"], json!(false));
}

#[test]
fn malformed_json_slot_keeps_inventory_incomplete() {
    let dir = TempDir::new();
    seed_complete_inventory(dir.path());
    fs::write(dir.path().join("quality.json"), b"not-json").unwrap();
    let output = run(dir.path(), CANDIDATE);
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["inventory_complete"], json!(false));
    let quality = report["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == json!("quality.json"))
        .unwrap();
    assert_eq!(quality["present"], json!(true));
    assert_eq!(quality["syntax_valid"], json!(false));
}
