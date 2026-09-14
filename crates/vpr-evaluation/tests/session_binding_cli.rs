use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::{Value, json};

static SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("vpr-session-binding-{}-{seq}", std::process::id()));
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

fn snapshot(session: u64) -> Value {
    json!({
        "schema_version":"rt0-owner-lab-session-evidence-0.1",
        "scope":"browser_observed_media_plane_only",
        "session_sequence":session,
        "canonical_playback_proven":false,
        "av_sync_proven":false,
        "voice_attempts":[{
            "request_sequence":1,
            "canonical_turn_sequence":10 + session,
            "status":"completed",
            "failure_code":null,
            "stt_millis":100,
            "llm_millis":200,
            "avatar_millis":50,
            "server_total_millis":350,
            "stt_usage":{"input_units":10,"output_units":null,"estimated_cost_microunits":2,"provider_charge_microunits":3},
            "llm_usage":{"input_units":10,"output_units":4,"estimated_cost_microunits":5,"provider_charge_microunits":7}
        }],
        "media_events":[
            {"request_sequence":1,"kind":"audio_started","elapsed_millis":400},
            {"request_sequence":null,"kind":"video_ready","elapsed_millis":500}
        ]
    })
}

#[test]
fn bind_mode_emits_exact_candidate_and_provider_state_receipt() {
    let dir = TempDir::new();
    let provider = dir.file("provider.json", &provider_state());
    let one = dir.file("one.json", &snapshot(1));
    let two = dir.file("two.json", &snapshot(2));
    let output = Command::new(env!("CARGO_BIN_EXE_vpr-rt0-session-aggregate"))
        .arg("bind")
        .arg(provider)
        .arg("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        .args([one, two])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value["schema_version"],
        "rt0-owner-lab-session-aggregate-binding-0.1"
    );
    assert_eq!(
        value["candidate_sha"],
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(value["snapshot_sha256"].as_array().unwrap().len(), 2);
    assert_eq!(value["aggregate"]["sessions"], 2);
    assert_eq!(value["aggregate"]["canonical_playback_proven"], false);
}

#[test]
fn bind_mode_fails_closed_for_bad_candidate() {
    let dir = TempDir::new();
    let provider = dir.file("provider.json", &provider_state());
    let one = dir.file("one.json", &snapshot(1));
    let output = Command::new(env!("CARGO_BIN_EXE_vpr-rt0-session-aggregate"))
        .arg("bind")
        .arg(provider)
        .arg("bad-sha")
        .arg(one)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("BINDING_INVALID"));
}
