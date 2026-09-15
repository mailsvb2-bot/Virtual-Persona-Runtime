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
    seed_bound_runtime_evidence(dir, &provider_digest);
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

fn seed_bound_runtime_evidence(dir: &Path, provider_digest: &str) {
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
    let bound_session = json!({
        "schema_version":"rt0-owner-lab-session-aggregate-binding-0.3",
        "candidate_sha":CANDIDATE,
        "provider_state_sha256":provider_digest,
        "snapshot_sha256":["9".repeat(64)],
        "aggregate":{
            "schema_version":"rt0-owner-lab-session-aggregate-0.3",
            "source_schema_version":"rt0-owner-lab-session-evidence-0.3",
            "sessions":1,
            "completed_voice_attempts":1,
            "failed_voice_attempts":0,
            "canonical_playback_proven":false,
            "av_sync_proven":false,
            "av_sync_absolute_offset":null,
            "stt_latency":null,
            "llm_latency":null,
            "avatar_submit_latency":null,
            "server_total_latency":null,
            "first_meaningful_audio":null,
            "interruption_stop":null,
            "first_useful_video":null,
            "recoverable_reconnect":null,
            "estimated_cost_microunits":null,
            "provider_charge_microunits":null
        }
    });
    fs::write(
        dir.join("bound-session-aggregate.json"),
        serde_json::to_vec_pretty(&bound_session).unwrap(),
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
