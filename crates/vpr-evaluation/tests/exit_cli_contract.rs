use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use vpr_evaluation::sha256_hex;

const CANDIDATE: &str = "1111111111111111111111111111111111111111";
const RELEASE_SPEC: &[u8] = b"rt0 release spec cli contract";
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
            "vpr-exit-cli-{}-{nanos}-{sequence}",
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

fn digest(byte: char) -> String {
    byte.to_string().repeat(64)
}

fn provider_state() -> Value {
    json!({
        "schema_version": "rt0-provider-state-0.1",
        "providers": [
            {"role":"stt","provider":"stt","model_or_representation":"v1","configuration_fingerprint_sha256":digest('a')},
            {"role":"llm","provider":"llm","model_or_representation":"v1","configuration_fingerprint_sha256":digest('b')},
            {"role":"avatar","provider":"avatar","model_or_representation":"v1","configuration_fingerprint_sha256":digest('c')}
        ]
    })
}

fn golden_report(provider_state_sha256: &str) -> Value {
    json!({
        "evidence_input_sha256": digest('f'),
        "binding": {
            "schema_version": "rt0-evidence-binding-0.1",
            "candidate_sha": CANDIDATE,
            "release_spec_sha256": sha256_hex(RELEASE_SPEC),
            "suite_sha256": digest('e'),
            "provider_state_sha256": provider_state_sha256
        },
        "provider_state": provider_state(),
        "golden": {
            "schema_version": "rt0-golden-0.1",
            "suite_id": "rt0.cli.contract",
            "total": 1,
            "passed": 1,
            "failed": 0,
            "cases": [{"case_id":"golden.pass","passed":true,"failures":[]}]
        }
    })
}

fn exit_evidence(golden_bytes: &[u8], provider_state_sha256: &str) -> Value {
    let distribution = |p50, p95| json!({"samples":10,"p50":p50,"p95":p95});
    let conversation = |role: &str, interruption: &str| {
        json!({
            "origin":"real","role":role,"russian":"passed","voice":"passed","video":"passed",
            "completed_turns":2,"interruption_exercised":interruption,"artifact_sha256":digest('a')
        })
    };
    json!({
        "schema_version":"rt0-exit-evidence-0.1",
        "candidate_sha":CANDIDATE,
        "release_spec_sha256":sha256_hex(RELEASE_SPEC),
        "golden_report_sha256":sha256_hex(golden_bytes),
        "provider_state_sha256":provider_state_sha256,
        "automated":{
            "ci":{"status":"passed","artifact_sha256":digest('a')},
            "e2e":{"status":"passed","artifact_sha256":digest('a')}
        },
        "conversations":{
            "owner":conversation("owner","passed"),
            "visitor":conversation("visitor","failed")
        },
        "acceptance":{
            "origin":"real","owner_happy_path":"passed","visitor_happy_path":"passed",
            "correction_path":"passed","failure_recovery_path":"passed","revoke_deny_path":"passed",
            "artifact_sha256":digest('a')
        },
        "quality":{
            "origin":"real",
            "text_first_meaningful_response":distribution(900,2400),
            "first_meaningful_audio":distribution(1400,2900),
            "interruption_stop":distribution(250,450),
            "first_useful_video":distribution(1200,2400),
            "av_sync_absolute_offset":distribution(50,110),
            "recoverable_reconnect":distribution(2000,4900),
            "artifact_sha256":digest('a')
        },
        "cost":{
            "origin":"real","measured_duration_millis":60000,"measured_cost_microunits":1234,
            "provider_charge_microunits":null,"artifact_sha256":digest('a')
        },
        "privacy_permissions":{
            "origin":"real","permission_suite":"passed","accepted_private_context_leakage":0,
            "accepted_false_owner_attribution":0,"revocation":"passed","egress_denial":"passed",
            "artifact_sha256":digest('a')
        },
        "human_evaluation":{
            "origin":"real","rubric_version":"rt0-human-v1","reviewer_count":1,
            "dimensions":{"voice_similarity":"recorded","voice_naturalness":"recorded",
                "appearance_plausibility":"recorded","persona_similarity":"recorded",
                "conversation_naturalness":"recorded"},
            "usable_for_continuation":"passed","artifact_sha256":digest('a')
        },
        "known_limitations":{"review_status":"passed","document_sha256":digest('a')}
    })
}

fn prepare(
    evidence_mutator: impl FnOnce(&mut Value),
) -> (TempDir, PathBuf, PathBuf, PathBuf, PathBuf) {
    let dir = TempDir::new();
    let golden_path = dir.path().join("golden-report.json");
    let provider_state_path = dir.path().join("provider-state.json");
    let evidence_path = dir.path().join("exit-evidence.json");
    let spec_path = dir.path().join("RT0_RELEASE_SPEC.md");
    let provider_state = provider_state();
    let provider_state_bytes = serde_json::to_vec_pretty(&provider_state).unwrap();
    let provider_state_sha256 = sha256_hex(&provider_state_bytes);
    let golden = golden_report(&provider_state_sha256);
    let golden_bytes = serde_json::to_vec_pretty(&golden).unwrap();
    let mut evidence = exit_evidence(&golden_bytes, &provider_state_sha256);
    evidence_mutator(&mut evidence);
    fs::write(&golden_path, golden_bytes).unwrap();
    fs::write(&provider_state_path, provider_state_bytes).unwrap();
    fs::write(
        &evidence_path,
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    fs::write(&spec_path, RELEASE_SPEC).unwrap();
    (
        dir,
        evidence_path,
        golden_path,
        provider_state_path,
        spec_path,
    )
}

fn run(
    evidence: &Path,
    golden: &Path,
    provider_state: &Path,
    spec: &Path,
    candidate: &str,
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_vpr-rt0-exit-evidence"))
        .arg(evidence)
        .arg(golden)
        .arg(provider_state)
        .arg(spec)
        .arg(candidate)
        .output()
        .unwrap()
}

#[test]
fn cli_returns_zero_only_for_complete_exact_bound_evidence() {
    let (_dir, evidence, golden, provider_state, spec) = prepare(|_| {});
    let output = run(&evidence, &golden, &provider_state, &spec, CANDIDATE);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("\"ready\": true"));
    assert!(!stdout.contains("prompt"));
    assert!(!stdout.contains("response_text"));
}

#[test]
fn cli_returns_one_for_valid_but_privacy_failing_evidence() {
    let (_dir, evidence, golden, provider_state, spec) = prepare(|value| {
        value["privacy_permissions"]["accepted_private_context_leakage"] = json!(1);
    });
    let output = run(&evidence, &golden, &provider_state, &spec, CANDIDATE);
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("PRIVATE_CONTEXT_LEAKAGE_ACCEPTED"));
    assert!(stdout.contains("\"ready\": false"));
}

#[test]
fn cli_rejects_tampered_provider_state_even_when_golden_report_is_valid_json() {
    let (_dir, evidence, golden, provider_state, spec) = prepare(|_| {});
    let mut state: Value = serde_json::from_slice(&fs::read(&provider_state).unwrap()).unwrap();
    state["providers"][0]["provider"] = json!("tampered-stt");
    fs::write(&provider_state, serde_json::to_vec_pretty(&state).unwrap()).unwrap();
    let output = run(&evidence, &golden, &provider_state, &spec, CANDIDATE);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("PROVIDER_STATE_DIGEST_MISMATCH")
    );
}

#[test]
fn cli_returns_two_for_stale_candidate_or_unknown_fields() {
    let (_dir, evidence, golden, provider_state, spec) = prepare(|_| {});
    let output = run(&evidence, &golden, &provider_state, &spec, &"2".repeat(40));
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("CANDIDATE_SHA_MISMATCH")
    );

    let (_dir, evidence, golden, provider_state, spec) = prepare(|value| {
        value["api_key"] = json!("must-not-be-accepted");
    });
    let output = run(&evidence, &golden, &provider_state, &spec, CANDIDATE);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("INPUT_INVALID")
    );
}
