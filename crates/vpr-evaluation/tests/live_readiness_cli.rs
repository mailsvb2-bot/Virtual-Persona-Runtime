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
        let path =
            std::env::temp_dir().join(format!("vpr-live-readiness-{}-{seq}", std::process::id()));
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
            {"role":"avatar","provider":"local-open-source","model_or_representation":"avatar-representation","configuration_fingerprint_sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}
        ]
    })
}

fn snapshot(first_audio: u64) -> Value {
    json!({
        "schema_version":"rt0-owner-lab-session-evidence-1.0",
        "scope":"browser_observed_media_plane_only",
        "session_sequence":1,
        "participant_role":"owner",
        "session_duration_millis":30000,
        "canonical_playback_proven":true,
        "av_sync_proven":true,
        "text_attempts":[{
            "request_sequence":1,
            "canonical_turn_sequence":11,
            "canonical_output_sequence":21,
            "status":"completed",
            "failure_code":null,
            "first_meaningful_response_millis":1000,
            "server_total_millis":1200,
            "llm_usage":{"input_units":8,"output_units":3,"estimated_cost_microunits":2,"provider_charge_microunits":3}
        }],
        "voice_attempts":[{
            "request_sequence":1,
            "canonical_turn_sequence":31,
            "canonical_output_sequence":41,
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
            {"request_sequence":1,"kind":"audio_started","elapsed_millis":first_audio},
            {"request_sequence":1,"kind":"interruption_stopped","elapsed_millis":500},
            {"request_sequence":null,"kind":"video_ready","elapsed_millis":2500},
            {"request_sequence":null,"kind":"reconnect_restored","elapsed_millis":5000}
        ],
        "av_sync_samples":[
            {"request_sequence":1,"sample_sequence":1,"reference":"web_rtc_estimated_playout_timestamp","absolute_offset_millis":120},
            {"request_sequence":1,"sample_sequence":2,"reference":"web_rtc_estimated_playout_timestamp","absolute_offset_millis":120},
            {"request_sequence":1,"sample_sequence":3,"reference":"web_rtc_estimated_playout_timestamp","absolute_offset_millis":120}
        ],
        "av_sync_diagnostics":[]
    })
}

fn bind(dir: &TempDir, snapshot_value: Value) -> (PathBuf, PathBuf, PathBuf) {
    let provider = dir.file("provider.json", &provider_state());
    let session = dir.file("session.json", &snapshot_value);
    let output = Command::new(env!("CARGO_BIN_EXE_vpr-rt0-session-aggregate"))
        .arg("bind")
        .arg(&provider)
        .arg(CANDIDATE)
        .arg(&session)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bound = dir.0.join("bound.json");
    fs::write(&bound, output.stdout).unwrap();
    (provider, session, bound)
}

fn readiness(provider: &PathBuf, bound: &PathBuf, session: &PathBuf) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_vpr-rt0-live-readiness"))
        .arg(provider)
        .arg(bound)
        .arg(CANDIDATE)
        .arg(session)
        .output()
        .unwrap()
}

#[test]
fn exact_release_spec_runtime_boundaries_report_ready() {
    let dir = TempDir::new();
    let (provider, session, bound) = bind(&dir, snapshot(1500));
    let output = readiness(&provider, &bound, &session);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema_version"], "rt0-live-readiness-report-0.1");
    assert_eq!(report["runtime_quality_ready"], true);
    assert_eq!(report["rerun_required"], false);
    assert_eq!(report["blockers"], json!([]));
}

#[test]
fn live_audio_threshold_failure_is_actionable() {
    let dir = TempDir::new();
    let (provider, session, bound) = bind(&dir, snapshot(1501));
    let output = readiness(&provider, &bound, &session);
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["runtime_quality_ready"], false);
    assert_eq!(report["rerun_required"], true);
    assert!(
        report["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "first_audio_exceeded")
    );
}

#[test]
fn detached_bound_aggregate_is_rejected() {
    let dir = TempDir::new();
    let (provider, session, bound) = bind(&dir, snapshot(1500));
    let mut value: Value = serde_json::from_slice(&fs::read(&bound).unwrap()).unwrap();
    value["aggregate"]["session_duration_millis"] = json!(29_999);
    fs::write(&bound, serde_json::to_vec(&value).unwrap()).unwrap();

    let output = readiness(&provider, &bound, &session);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("bound session aggregate does not match the supplied raw snapshots")
    );
}
