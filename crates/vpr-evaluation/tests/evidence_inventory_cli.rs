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
    seed_exit_and_supporting(dir, &provider_digest);
}

fn claim_bytes(claim: &Value) -> Vec<u8> {
    let mut claim = claim.clone();
    claim
        .as_object_mut()
        .unwrap()
        .remove("artifact_sha256")
        .unwrap();
    serde_json::to_vec_pretty(&claim).unwrap()
}

fn bind_projected_claim(
    dir: &Path,
    name: &str,
    claim: &mut Value,
    candidate_sha: &str,
    provider_state_sha256: Option<&str>,
) {
    let mut projected: Value = serde_json::from_slice(&claim_bytes(claim)).unwrap();
    projected["candidate_sha"] = json!(candidate_sha);
    if let Some(provider_state_sha256) = provider_state_sha256 {
        projected["provider_state_sha256"] = json!(provider_state_sha256);
    }
    let bytes = serde_json::to_vec_pretty(&projected).unwrap();
    fs::write(dir.join(name), &bytes).unwrap();
    claim["artifact_sha256"] = json!(sha256_hex(&bytes));
}

fn seed_exit_and_supporting(dir: &Path, provider_digest: &str) {
    let mut evidence: Value = serde_json::from_str(include_str!(
        "../../../docs/evaluation/rt0_exit_evidence.synthetic.example.json"
    ))
    .unwrap();
    evidence["candidate_sha"] = json!(CANDIDATE);
    evidence["release_spec_sha256"] = json!(sha256_hex(RELEASE_SPEC));
    evidence["golden_report_sha256"] = json!(sha256_hex(
        &fs::read(dir.join("bound-golden-report.json")).unwrap()
    ));
    evidence["provider_state_sha256"] = json!(provider_digest);
    evidence["live_provider_probe_sha256"] = json!(sha256_hex(
        &fs::read(dir.join("provider-probe.json")).unwrap()
    ));
    evidence["conversation_attempt_sha256"] = json!(sha256_hex(
        &fs::read(dir.join("conversation-attempt.json")).unwrap()
    ));
    evidence["bound_session_aggregate_sha256"] = json!(sha256_hex(
        &fs::read(dir.join("bound-session-aggregate.json")).unwrap()
    ));

    bind_projected_claim(
        dir,
        "ci-evidence.json",
        &mut evidence["automated"]["ci"],
        CANDIDATE,
        None,
    );
    bind_projected_claim(
        dir,
        "e2e-evidence.json",
        &mut evidence["automated"]["e2e"],
        CANDIDATE,
        None,
    );
    for (name, pointer) in [
        ("owner-conversation.json", "/conversations/owner"),
        ("visitor-conversation.json", "/conversations/visitor"),
        ("acceptance.json", "/acceptance"),
        ("quality.json", "/quality"),
        ("cost.json", "/cost"),
        ("privacy-permissions.json", "/privacy_permissions"),
        ("human-evaluation.json", "/human_evaluation"),
    ] {
        let claim = evidence.pointer_mut(pointer).unwrap();
        bind_projected_claim(dir, name, claim, CANDIDATE, Some(provider_digest));
    }
    let limitations = b"reviewed RT0 inventory limitations\n";
    fs::write(dir.join("known-limitations.md"), limitations).unwrap();
    evidence["known_limitations"]["document_sha256"] = json!(sha256_hex(limitations));
    fs::write(
        dir.join("exit-evidence.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
}

fn seed_bound_runtime_evidence(dir: &Path, provider_digest: &str, provider_state_bytes: &[u8]) {
    let conversation = json!({
        "schema_version":"rt0-live-conversation-attempt-0.1",
        "candidate_sha":CANDIDATE,
        "provider_state_sha256":provider_digest,
        "profile_input_sha256":"3".repeat(64),
        "persona_id_sha256":"4".repeat(64),
        "persona_version":2,
        "reviewed_claims":1,
        "owner":{
            "audience":"owner",
            "input_audio_sha256":"5".repeat(64),
            "transcript_sha256":"6".repeat(64),
            "transcript_chars":12,
            "reply_sha256":"7".repeat(64),
            "reply_chars":18,
            "locale":"ru"
        },
        "visitor":{
            "audience":"visitor",
            "input_audio_sha256":"8".repeat(64),
            "transcript_sha256":"9".repeat(64),
            "transcript_chars":10,
            "reply_sha256":"a".repeat(64),
            "reply_chars":16,
            "locale":"ru-RU"
        },
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

    let snapshot = |role: &str, session_sequence: u64, interruption: bool| {
        let mut media = vec![
            json!({"request_sequence":1,"kind":"audio_started","elapsed_millis":500}),
            json!({"request_sequence":null,"kind":"video_ready","elapsed_millis":700}),
        ];
        if interruption {
            media.push(json!({
                "request_sequence":1,
                "kind":"interruption_stopped",
                "elapsed_millis":250
            }));
        }
        json!({
            "schema_version":"rt0-owner-lab-session-evidence-0.4",
            "scope":"browser_observed_media_plane_only",
            "session_sequence":session_sequence,
            "participant_role":role,
            "canonical_playback_proven":true,
            "av_sync_proven":false,
            "voice_attempts":[{
                "request_sequence":1,
                "canonical_turn_sequence":10 + session_sequence,
                "canonical_output_sequence":20 + session_sequence,
                "canonical_playback_confirmed":true,
                "status":"completed",
                "failure_code":null,
                "stt_millis":100,
                "llm_millis":120,
                "avatar_millis":150,
                "server_total_millis":370,
                "stt_usage":{"input_units":1,"output_units":0,"estimated_cost_microunits":1,"provider_charge_microunits":null},
                "llm_usage":{"input_units":1,"output_units":1,"estimated_cost_microunits":1,"provider_charge_microunits":null}
            }],
            "media_events":media,
            "av_sync_samples":[]
        })
    };
    let owner_bytes = serde_json::to_vec_pretty(&snapshot("owner", 1, true)).unwrap();
    let visitor_bytes = serde_json::to_vec_pretty(&snapshot("visitor", 2, false)).unwrap();
    fs::write(dir.join("session-owner.json"), &owner_bytes).unwrap();
    fs::write(dir.join("session-visitor.json"), &visitor_bytes).unwrap();
    let bound = bind_owner_lab_session_evidence(
        &[owner_bytes.as_slice(), visitor_bytes.as_slice()],
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
    assert_eq!(report["bindings"]["exit_candidate_matches"], json!(true));
    assert_eq!(
        report["bindings"]["exit_release_spec_matches_golden"],
        json!(true)
    );
    assert_eq!(
        report["bindings"]["exit_golden_report_digest_matches"],
        json!(true)
    );
    assert_eq!(
        report["bindings"]["exit_provider_state_matches"],
        json!(true)
    );
    assert_eq!(report["bindings"]["exit_probe_digest_matches"], json!(true));
    assert_eq!(
        report["bindings"]["exit_conversation_digest_matches"],
        json!(true)
    );
    assert_eq!(
        report["bindings"]["exit_session_digest_matches"],
        json!(true)
    );
    assert_eq!(
        report["bindings"]["supporting_artifacts_bound"],
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
    assert_eq!(
        report["schema_version"],
        json!("rt0-evidence-inventory-0.4")
    );
    assert_eq!(report["session_snapshots"]["expected"], json!(2));
    assert_eq!(report["session_snapshots"]["discovered"], json!(2));
    assert_eq!(
        report["session_snapshots"]["all_expected_present"],
        json!(true)
    );
    assert_eq!(
        report["session_snapshots"]["no_unbound_snapshots"],
        json!(true)
    );
    assert_eq!(
        report["session_snapshots"]["binding_recomputed"],
        json!(true)
    );
    assert_eq!(
        report["session_snapshots"]["conversation_claims_bound"],
        json!(true)
    );
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
    assert_eq!(report["session_snapshots"]["expected"], json!(2));
    assert_eq!(report["session_snapshots"]["discovered"], json!(1));
    assert_eq!(
        report["session_snapshots"]["all_expected_present"],
        json!(false)
    );
    assert_eq!(
        report["session_snapshots"]["binding_recomputed"],
        json!(false)
    );
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
    assert_eq!(
        report["session_snapshots"]["all_expected_present"],
        json!(false)
    );
    assert_eq!(
        report["session_snapshots"]["no_unbound_snapshots"],
        json!(false)
    );
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
    assert_eq!(report["session_snapshots"]["expected"], json!(2));
    assert_eq!(report["session_snapshots"]["discovered"], json!(3));
    assert_eq!(
        report["session_snapshots"]["no_unbound_snapshots"],
        json!(false)
    );
    assert_eq!(
        report["session_snapshots"]["binding_recomputed"],
        json!(false)
    );
}

#[test]
fn extra_valid_session_snapshot_keeps_inventory_incomplete() {
    let dir = TempDir::new();
    seed_complete_inventory(dir.path());
    let mut snapshot: Value =
        serde_json::from_slice(&fs::read(dir.path().join("session-owner.json")).unwrap()).unwrap();
    snapshot["session_sequence"] = json!(3);
    fs::write(
        dir.path().join("session-stale.json"),
        serde_json::to_vec_pretty(&snapshot).unwrap(),
    )
    .unwrap();
    let output = run(dir.path(), CANDIDATE);
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["inventory_complete"], json!(false));
    assert_eq!(report["session_snapshots"]["expected"], json!(2));
    assert_eq!(report["session_snapshots"]["discovered"], json!(3));
    assert_eq!(
        report["session_snapshots"]["no_unbound_snapshots"],
        json!(false)
    );
}

#[test]
fn rehashed_role_forgery_keeps_inventory_incomplete() {
    let dir = TempDir::new();
    seed_complete_inventory(dir.path());
    let owner_path = dir.path().join("session-owner.json");
    let visitor_path = dir.path().join("session-visitor.json");
    let mut owner: Value = serde_json::from_slice(&fs::read(&owner_path).unwrap()).unwrap();
    owner["participant_role"] = json!("visitor");
    let owner_bytes = serde_json::to_vec_pretty(&owner).unwrap();
    fs::write(&owner_path, &owner_bytes).unwrap();
    let visitor_bytes = fs::read(&visitor_path).unwrap();
    let provider_state_bytes = fs::read(dir.path().join("provider-state.json")).unwrap();
    let bound = bind_owner_lab_session_evidence(
        &[owner_bytes.as_slice(), visitor_bytes.as_slice()],
        &provider_state_bytes,
        CANDIDATE,
    )
    .unwrap();
    let bound_bytes = serde_json::to_vec_pretty(&bound).unwrap();
    fs::write(dir.path().join("bound-session-aggregate.json"), &bound_bytes).unwrap();
    let exit_path = dir.path().join("exit-evidence.json");
    let mut exit: Value = serde_json::from_slice(&fs::read(&exit_path).unwrap()).unwrap();
    exit["bound_session_aggregate_sha256"] = json!(sha256_hex(&bound_bytes));
    fs::write(&exit_path, serde_json::to_vec_pretty(&exit).unwrap()).unwrap();

    let output = run(dir.path(), CANDIDATE);
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["inventory_complete"], json!(false));
    assert_eq!(report["session_snapshots"]["binding_recomputed"], json!(true));
    assert_eq!(
        report["session_snapshots"]["conversation_claims_bound"],
        json!(false)
    );
}

#[test]
fn detached_completed_turn_claim_keeps_inventory_incomplete() {
    let dir = TempDir::new();
    seed_complete_inventory(dir.path());
    let exit_path = dir.path().join("exit-evidence.json");
    let mut exit: Value = serde_json::from_slice(&fs::read(&exit_path).unwrap()).unwrap();
    exit["conversations"]["owner"]["completed_turns"] = json!(2);
    let provider_digest = sha256_hex(&fs::read(dir.path().join("provider-state.json")).unwrap());
    let owner_claim = exit.pointer_mut("/conversations/owner").unwrap();
    bind_projected_claim(
        dir.path(),
        "owner-conversation.json",
        owner_claim,
        CANDIDATE,
        Some(provider_digest.as_str()),
    );
    fs::write(&exit_path, serde_json::to_vec_pretty(&exit).unwrap()).unwrap();

    let output = run(dir.path(), CANDIDATE);
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["inventory_complete"], json!(false));
    assert_eq!(
        report["bindings"]["supporting_artifacts_bound"],
        json!(true)
    );
    assert_eq!(
        report["session_snapshots"]["conversation_claims_bound"],
        json!(false)
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
    assert_eq!(report["bindings"]["exit_candidate_matches"], json!(false));
}

#[test]
fn rehashed_stale_supporting_claim_keeps_inventory_incomplete() {
    let dir = TempDir::new();
    seed_complete_inventory(dir.path());
    let quality_path = dir.path().join("quality.json");
    let mut quality: Value = serde_json::from_slice(&fs::read(&quality_path).unwrap()).unwrap();
    quality["provider_state_sha256"] = json!("2".repeat(64));
    let quality_bytes = serde_json::to_vec_pretty(&quality).unwrap();
    fs::write(&quality_path, &quality_bytes).unwrap();
    let exit_path = dir.path().join("exit-evidence.json");
    let mut exit: Value = serde_json::from_slice(&fs::read(&exit_path).unwrap()).unwrap();
    exit["quality"]["artifact_sha256"] = json!(sha256_hex(&quality_bytes));
    fs::write(&exit_path, serde_json::to_vec_pretty(&exit).unwrap()).unwrap();

    let output = run(dir.path(), CANDIDATE);
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["inventory_complete"], json!(false));
    assert_eq!(
        report["bindings"]["supporting_artifacts_bound"],
        json!(false)
    );
}

#[test]
fn stale_exit_manifest_digest_keeps_inventory_incomplete() {
    let dir = TempDir::new();
    seed_complete_inventory(dir.path());
    let exit_path = dir.path().join("exit-evidence.json");
    let mut exit: Value = serde_json::from_slice(&fs::read(&exit_path).unwrap()).unwrap();
    exit["live_provider_probe_sha256"] = json!("0".repeat(64));
    fs::write(&exit_path, serde_json::to_vec_pretty(&exit).unwrap()).unwrap();

    let output = run(dir.path(), CANDIDATE);
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["inventory_complete"], json!(false));
    assert_eq!(
        report["bindings"]["exit_probe_digest_matches"],
        json!(false)
    );
    assert_eq!(
        report["bindings"]["supporting_artifacts_bound"],
        json!(true)
    );
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
