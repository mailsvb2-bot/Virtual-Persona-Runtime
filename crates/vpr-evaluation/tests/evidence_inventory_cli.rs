mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use vpr_evaluation::{bind_owner_lab_session_evidence, sha256_hex};

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
    seed_bound_runtime_evidence(dir, &provider_digest, &fixture.provider_state_bytes);
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

fn seed_bound_runtime_evidence(
    dir: &Path,
    provider_digest: &str,
    provider_state_bytes: &[u8],
) {
    let conversation = json!({
        "schema_version":"rt0-live-conversation-attempt-0.1",
        "candidate_sha":CANDIDATE,
        "provider_state_sha256":provider_digest,
        "profile_input_sha256":"3".repeat(64),
        "persona_id_sha256":"4".repeat(64),
        "persona_version":2,
        "reviewed_claims":1,
        "owner":{},
        "visitor":{},
        "conversation_attempted":true,
        "provider_output_submitted":true,
        "browser_media_playback":"not_proven",
        "video_render":"not_proven",
        "human_review":"not_proven"
    });
    fs::write(
        dir.join("conversation-attempt.json"),
        serde_json::to_vec_pretty(&conversation).unwrap(),
    )
    .unwrap();

    let snapshot = json!({
        "schema_version":"rt0-owner-lab-session-evidence-0.3",
        "scope":"browser_observed_media_plane_only",
        "session_sequence":1,
        "canonical_playback_proven":false,
        "av_sync_proven":false,
        "voice_attempts":[],
        "media_events":[],
        "av_sync_samples":[]
    });
    let snapshot_bytes = serde_json::to_vec_pretty(&snapshot).unwrap();
    fs::write(dir.join("session-owner.json"), &snapshot_bytes).unwrap();
    let bound = bind_owner_lab_session_evidence(
        &[snapshot_bytes.as_slice()],
        provider_state_bytes,
        CANDIDATE,
    )
    .unwrap();
    fs::write(
        dir.join("bound-session-aggregate.json"),
        serde_json::to_vec_pretty(&bound).unwrap(),
    )
    .unwrap();
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
    assert_eq!(
        report["bindings"]["conversation_candidate_matches"],
        json!(true)
    );
    assert_eq!(
        report["bindings"]["conversation_provider_state_matches"],
        json!(true)
    );
    assert_eq!(report["bindings"]["session_candidate_matches"], json!(true));
    assert_eq!(
        report["bindings"]["session_provider_state_matches"],
        json!(true)
    );
    assert_eq!(report["schema_version"], json!("rt0-evidence-inventory-0.2"));
    assert_eq!(report["session_snapshots"]["expected"], json!(1));
    assert_eq!(report["session_snapshots"]["discovered"], json!(1));
    assert_eq!(report["session_snapshots"]["all_expected_present"], json!(true));
    assert_eq!(report["session_snapshots"]["no_unbound_snapshots"], json!(true));
    assert_eq!(report["session_snapshots"]["binding_recomputed"], json!(true));
}

#[test]
fn missing_raw_session_snapshot_keeps_inventory_incomplete() {
    let dir = TempDir::new();
    seed_complete_inventory(dir.path());
    fs::remove_file(dir.path().join("session-owner.json")).unwrap();
    let output = run(dir.path(), CANDIDATE);
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["inventory_complete"], json!(false));
    assert_eq!(report["session_snapshots"]["expected"], json!(1));
    assert_eq!(report["session_snapshots"]["discovered"], json!(0));
    assert_eq!(report["session_snapshots"]["all_expected_present"], json!(false));
    assert_eq!(report["session_snapshots"]["binding_recomputed"], json!(false));
}

#[test]
fn tampered_raw_session_snapshot_keeps_inventory_incomplete() {
    let dir = TempDir::new();
    seed_complete_inventory(dir.path());
    let path = dir.path().join("session-owner.json");
    let mut snapshot: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    snapshot["session_sequence"] = json!(2);
    fs::write(&path, serde_json::to_vec_pretty(&snapshot).unwrap()).unwrap();
    let output = run(dir.path(), CANDIDATE);
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["inventory_complete"], json!(false));
    assert_eq!(report["session_snapshots"]["all_expected_present"], json!(false));
    assert_eq!(report["session_snapshots"]["no_unbound_snapshots"], json!(false));
}

#[test]
fn duplicate_raw_session_snapshot_keeps_inventory_incomplete() {
    let dir = TempDir::new();
    seed_complete_inventory(dir.path());
    let bytes = fs::read(dir.path().join("session-owner.json")).unwrap();
    fs::write(dir.path().join("session-copy.json"), bytes).unwrap();
    let output = run(dir.path(), CANDIDATE);
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["inventory_complete"], json!(false));
    assert_eq!(report["session_snapshots"]["expected"], json!(1));
    assert_eq!(report["session_snapshots"]["discovered"], json!(2));
    assert_eq!(report["session_snapshots"]["no_unbound_snapshots"], json!(false));
    assert_eq!(report["session_snapshots"]["binding_recomputed"], json!(false));
}

#[test]
fn extra_valid_session_snapshot_keeps_inventory_incomplete() {
    let dir = TempDir::new();
    seed_complete_inventory(dir.path());
    let mut snapshot: Value = serde_json::from_slice(
        &fs::read(dir.path().join("session-owner.json")).unwrap(),
    )
    .unwrap();
    snapshot["session_sequence"] = json!(2);
    fs::write(
        dir.path().join("session-stale.json"),
        serde_json::to_vec_pretty(&snapshot).unwrap(),
    )
    .unwrap();
    let output = run(dir.path(), CANDIDATE);
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["inventory_complete"], json!(false));
    assert_eq!(report["session_snapshots"]["expected"], json!(1));
    assert_eq!(report["session_snapshots"]["discovered"], json!(2));
    assert_eq!(report["session_snapshots"]["no_unbound_snapshots"], json!(false));
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

#[test]
fn conversation_and_session_binding_mismatch_keeps_inventory_incomplete() {
    let dir = TempDir::new();
    seed_complete_inventory(dir.path());

    let mut conversation: Value =
        serde_json::from_slice(&fs::read(dir.path().join("conversation-attempt.json")).unwrap())
            .unwrap();
    conversation["provider_state_sha256"] = json!("0".repeat(64));
    fs::write(
        dir.path().join("conversation-attempt.json"),
        serde_json::to_vec_pretty(&conversation).unwrap(),
    )
    .unwrap();

    let mut session: Value =
        serde_json::from_slice(&fs::read(dir.path().join("bound-session-aggregate.json")).unwrap())
            .unwrap();
    session["candidate_sha"] = json!("2".repeat(40));
    fs::write(
        dir.path().join("bound-session-aggregate.json"),
        serde_json::to_vec_pretty(&session).unwrap(),
    )
    .unwrap();

    let output = run(dir.path(), CANDIDATE);
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["inventory_complete"], json!(false));
    assert_eq!(
        report["bindings"]["conversation_provider_state_matches"],
        json!(false)
    );
    assert_eq!(
        report["bindings"]["session_candidate_matches"],
        json!(false)
    );
}
