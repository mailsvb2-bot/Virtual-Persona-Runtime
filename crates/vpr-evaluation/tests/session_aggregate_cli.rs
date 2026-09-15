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
        let path = std::env::temp_dir().join(format!(
            "vpr-session-aggregate-{}-{seq}",
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

fn snapshot(session: u64, elapsed: u64) -> Value {
    json!({
        "schema_version":"rt0-owner-lab-session-evidence-0.2",
        "scope":"browser_observed_media_plane_only",
        "session_sequence":session,
        "canonical_playback_proven":true,
        "av_sync_proven":false,
        "voice_attempts":[{
            "request_sequence":1,
            "canonical_turn_sequence":10 + session,
            "canonical_output_sequence":20 + session,
            "canonical_playback_confirmed":true,
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
            {"request_sequence":1,"kind":"audio_started","elapsed_millis":elapsed},
            {"request_sequence":null,"kind":"video_ready","elapsed_millis":elapsed + 100}
        ]
    })
}

#[test]
fn cli_aggregates_sanitized_snapshots() {
    let dir = TempDir::new();
    let one = dir.file("one.json", &snapshot(1, 400));
    let two = dir.file("two.json", &snapshot(2, 600));
    let output = Command::new(env!("CARGO_BIN_EXE_vpr-rt0-session-aggregate"))
        .args([one, two])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["sessions"], 2);
    assert_eq!(value["first_meaningful_audio"]["p50"], 400);
    assert_eq!(value["first_meaningful_audio"]["p95"], 600);
    assert_eq!(value["estimated_cost_microunits"], 14);
    assert_eq!(value["canonical_playback_proven"], true);
}

#[test]
fn cli_rejects_unknown_fields_and_duplicate_snapshots() {
    let dir = TempDir::new();
    let mut bad = snapshot(1, 400);
    bad["raw_transcript"] = json!("must never be accepted");
    let bad_path = dir.file("bad.json", &bad);
    let rejected = Command::new(env!("CARGO_BIN_EXE_vpr-rt0-session-aggregate"))
        .arg(bad_path)
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("INPUT_INVALID"));

    let same = dir.file("same.json", &snapshot(2, 500));
    let duplicate = Command::new(env!("CARGO_BIN_EXE_vpr-rt0-session-aggregate"))
        .args([&same, &same])
        .output()
        .unwrap();
    assert_eq!(duplicate.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("EVIDENCE_INVALID"));
}
