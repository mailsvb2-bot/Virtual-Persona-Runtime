use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::{Value, json};

static SEQ: AtomicU64 = AtomicU64::new(1);
const CANDIDATE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "vpr-supporting-prefill-{}-{seq}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn file(&self, name: &str, value: &Value) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn provider_state() -> Value {
    json!({
        "schema_version":"rt0-provider-state-0.1",
        "providers":[
            {"role":"stt","provider":"openai-transcription","model_or_representation":"stt-model","configuration_fingerprint_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            {"role":"llm","provider":"anthropic","model_or_representation":"llm-model","configuration_fingerprint_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
            {"role":"avatar","provider":"did-agents-streams","model_or_representation":"avatar-representation","configuration_fingerprint_sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}
        ]
    })
}

fn snapshot(session: u64, provider_state_sha256: &str) -> Value {
    json!({
        "schema_version":"rt0-owner-lab-session-evidence-1.1",
        "candidate_sha":CANDIDATE,
        "provider_state_sha256":provider_state_sha256,
        "scope":"browser_observed_media_plane_only",
        "session_sequence":session,
        "participant_role":if session == 1 { "owner" } else { "visitor" },
        "session_duration_millis":15000,
        "canonical_playback_proven":true,
        "av_sync_proven":false,
        "text_attempts":[{
            "request_sequence":1,
            "canonical_turn_sequence":5 + session,
            "canonical_output_sequence":15 + session,
            "status":"completed",
            "failure_code":null,
            "first_meaningful_response_millis":150,
            "server_total_millis":200,
            "llm_usage":{"input_units":8,"output_units":3,"estimated_cost_microunits":2,"provider_charge_microunits":3}
        }],
        "voice_attempts":[{
            "request_sequence":1,
            "canonical_turn_sequence":10 + session,
            "canonical_output_sequence":20 + session,
            "canonical_playback_confirmed":true,
            "status":"completed",
            "failure_code":null,
            "stt_millis":100,
            "llm_millis":200,
            "llm_first_meaningful_millis":120,
            "avatar_millis":50,
            "server_total_millis":350,
            "stt_usage":{"input_units":10,"output_units":null,"estimated_cost_microunits":2,"provider_charge_microunits":3},
            "llm_usage":{"input_units":10,"output_units":4,"estimated_cost_microunits":5,"provider_charge_microunits":7}
        }],
        "media_events":[
            {"request_sequence":1,"kind":"audio_started","elapsed_millis":400},
            {"request_sequence":null,"kind":"video_ready","elapsed_millis":500}
        ],
        "av_sync_samples":[]
    })
}

fn prepare_bound(dir: &TempDir) -> (PathBuf, PathBuf, PathBuf) {
    let provider_value = provider_state();
    let provider_bytes = serde_json::to_vec(&provider_value).unwrap();
    let provider_state_sha256 = vpr_evaluation::sha256_hex(&provider_bytes);
    let provider = dir.file("provider.json", &provider_value);
    let owner = dir.file("owner.json", &snapshot(1, &provider_state_sha256));
    let visitor = dir.file("visitor.json", &snapshot(2, &provider_state_sha256));
    let aggregate = Command::new(env!("CARGO_BIN_EXE_vpr-rt0-session-aggregate"))
        .arg("bind")
        .arg(&provider)
        .arg(CANDIDATE)
        .args([&owner, &visitor])
        .output()
        .unwrap();
    assert!(
        aggregate.status.success(),
        "{}",
        String::from_utf8_lossy(&aggregate.stderr)
    );
    let bound = dir.0.join("bound.json");
    fs::write(&bound, aggregate.stdout).unwrap();
    (provider, owner, visitor)
}

#[test]
fn prefill_recomputes_runtime_binding_and_remains_review_required() {
    let dir = TempDir::new();
    let (provider, owner, visitor) = prepare_bound(&dir);
    let bound = dir.0.join("bound.json");
    let output_path = dir.0.join("reviewed-observations.json");

    let output = Command::new(env!("CARGO_BIN_EXE_vpr-rt0-supporting-input-prefill"))
        .arg(&output_path)
        .arg(&provider)
        .arg(&bound)
        .arg(CANDIDATE)
        .args([&owner, &visitor])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&fs::read(output_path).unwrap()).unwrap();
    assert_eq!(
        value["schema_version"],
        "rt0-manual-supporting-observations-0.2"
    );
    assert_eq!(value["attestation"], "REVIEW_REQUIRED");
    assert_eq!(value["cost"]["measured_duration_millis"], 30_000);
    assert_eq!(value["cost"]["estimated_cost_microunits"], 18);
    assert_eq!(value["cost"]["provider_charge_microunits"], 26);
    assert_eq!(
        value["cost"]["estimated_cost_covered_provider_roles"],
        json!(["stt", "llm"])
    );
    assert_eq!(
        value["cost"]["provider_charge_covered_provider_roles"],
        json!(["stt", "llm"])
    );
    assert_eq!(value["acceptance"]["owner_happy_path"], "failed");
    assert_eq!(value["human_evaluation"]["reviewer_count"], 0);
    assert_eq!(
        value["human_evaluation"]["visitor_distinct_non_owner_human_verified"],
        "failed"
    );
}

#[test]
fn prefill_rejects_detached_bound_aggregate_even_when_json_is_valid() {
    let dir = TempDir::new();
    let (provider, owner, visitor) = prepare_bound(&dir);
    let bound_path = dir.0.join("bound.json");
    let mut bound: Value = serde_json::from_slice(&fs::read(&bound_path).unwrap()).unwrap();
    bound["aggregate"]["session_duration_millis"] = json!(29_999);
    fs::write(&bound_path, serde_json::to_vec(&bound).unwrap()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_vpr-rt0-supporting-input-prefill"))
        .arg(dir.0.join("reviewed-observations.json"))
        .arg(provider)
        .arg(bound_path)
        .arg(CANDIDATE)
        .args([owner, visitor])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("bound session aggregate does not match the supplied raw snapshots")
    );
}

#[test]
fn prefill_refuses_to_overwrite_operator_input() {
    let dir = TempDir::new();
    let (provider, owner, visitor) = prepare_bound(&dir);
    let output_path = dir.0.join("reviewed-observations.json");
    fs::write(&output_path, b"do not overwrite").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_vpr-rt0-supporting-input-prefill"))
        .arg(&output_path)
        .arg(provider)
        .arg(dir.0.join("bound.json"))
        .arg(CANDIDATE)
        .args([owner, visitor])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read(output_path).unwrap(), b"do not overwrite");
}
