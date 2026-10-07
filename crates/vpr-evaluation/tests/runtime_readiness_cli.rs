use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use vpr_evaluation::{ParticipantRole, bind_owner_lab_session_evidence, sha256_hex};

const CANDIDATE: &str = "1111111111111111111111111111111111111111";
static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "vpr-runtime-readiness-{label}-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn provider_state() -> Vec<u8> {
    include_bytes!("../../../docs/evaluation/rt0_provider_state.synthetic.example.json").to_vec()
}

fn digest(ch: char) -> String {
    ch.to_string().repeat(64)
}

fn conversation_attempt(provider_state_bytes: &[u8]) -> Vec<u8> {
    serde_json::to_vec_pretty(&json!({
        "schema_version":"rt0-live-conversation-attempt-0.1",
        "candidate_sha":CANDIDATE,
        "provider_state_sha256":sha256_hex(provider_state_bytes),
        "profile_input_sha256":digest('3'),
        "persona_id_sha256":digest('4'),
        "persona_version":2,
        "reviewed_claims":1,
        "owner":{
            "audience":"owner",
            "input_audio_sha256":digest('5'),
            "transcript_sha256":digest('6'),
            "transcript_chars":12,
            "reply_sha256":digest('7'),
            "reply_chars":18,
            "locale":"ru"
        },
        "visitor":{
            "audience":"visitor",
            "input_audio_sha256":digest('8'),
            "transcript_sha256":digest('9'),
            "transcript_chars":10,
            "reply_sha256":digest('a'),
            "reply_chars":16,
            "locale":"ru-RU"
        },
        "conversation_attempted":true,
        "provider_output_submitted":true,
        "browser_media_playback":"not_proven",
        "video_render":"not_proven",
        "human_review":"not_proven"
    }))
    .unwrap()
}

fn voice_attempt(
    request_sequence: u64,
    canonical_turn_sequence: u64,
    canonical_output_sequence: u64,
    canonical_playback_confirmed: bool,
) -> Value {
    json!({
        "request_sequence":request_sequence,
        "canonical_turn_sequence":canonical_turn_sequence,
        "canonical_output_sequence":canonical_output_sequence,
        "canonical_playback_confirmed":canonical_playback_confirmed,
        "status":"completed",
        "failure_code":null,
        "stt_millis":100,
        "llm_millis":120,
        "llm_first_meaningful_millis":80,
        "avatar_millis":150,
        "server_total_millis":370,
        "stt_usage":{
            "input_units":1,
            "output_units":0,
            "estimated_cost_microunits":1,
            "provider_charge_microunits":null
        },
        "llm_usage":{
            "input_units":1,
            "output_units":1,
            "estimated_cost_microunits":1,
            "provider_charge_microunits":null
        }
    })
}

fn snapshot(
    role: ParticipantRole,
    session_sequence: u64,
    first_audio_millis: u64,
    reconnect: bool,
    provider_state_sha256: &str,
) -> Vec<u8> {
    let mut voice_attempts = vec![voice_attempt(
        1,
        10 + session_sequence,
        20 + session_sequence,
        true,
    )];
    let mut media_events = vec![
        json!({
            "request_sequence":1,
            "kind":"audio_started",
            "elapsed_millis":first_audio_millis
        }),
        json!({
            "request_sequence":1,
            "kind":"playback_completed",
            "elapsed_millis":650
        }),
        json!({
            "request_sequence":null,
            "kind":"video_ready",
            "elapsed_millis":700
        }),
    ];
    if role == ParticipantRole::Owner {
        voice_attempts.push(voice_attempt(
            2,
            30 + session_sequence,
            40 + session_sequence,
            false,
        ));
        media_events.push(json!({
            "request_sequence":2,
            "kind":"audio_started",
            "elapsed_millis":first_audio_millis
        }));
        media_events.push(json!({
            "request_sequence":2,
            "kind":"interruption_stopped",
            "elapsed_millis":250
        }));
    }
    if reconnect {
        media_events.push(json!({
            "request_sequence":null,
            "kind":"reconnect_restored",
            "elapsed_millis":800
        }));
    }

    serde_json::to_vec_pretty(&json!({
        "schema_version":"rt0-owner-lab-session-evidence-1.3",
        "candidate_sha":CANDIDATE,
        "provider_state_sha256":provider_state_sha256,
        "scope":"browser_observed_media_plane_only",
        "session_sequence":session_sequence,
        "participant_role":role,
        "session_duration_millis":15000,
        "canonical_playback_proven":true,
        "av_sync_proven":true,
        "text_attempts":[{
            "request_sequence":1,
            "canonical_turn_sequence":5 + session_sequence,
            "canonical_output_sequence":15 + session_sequence,
            "status":"completed",
            "failure_code":null,
            "first_meaningful_response_millis":if role == ParticipantRole::Owner { 900 } else { 2400 },
            "server_total_millis":if role == ParticipantRole::Owner { 1000 } else { 2500 },
            "llm_usage":{
                "input_units":1,
                "output_units":1,
                "estimated_cost_microunits":1,
                "provider_charge_microunits":null
            }
        }],
        "voice_attempts":voice_attempts,
        "media_events":media_events,
        "av_sync_samples":[
            {"request_sequence":1,"sample_sequence":1,"reference":"web_rtc_estimated_playout_timestamp","absolute_offset_millis":40},
            {"request_sequence":1,"sample_sequence":2,"reference":"web_rtc_estimated_playout_timestamp","absolute_offset_millis":60},
            {"request_sequence":1,"sample_sequence":3,"reference":"web_rtc_estimated_playout_timestamp","absolute_offset_millis":110}
        ],
        "av_sync_diagnostics":[]
    }))
    .unwrap()
}

struct Fixture {
    root: PathBuf,
    provider: PathBuf,
    conversation: PathBuf,
    bound: PathBuf,
    owner: PathBuf,
    visitor: PathBuf,
}

impl Fixture {
    fn new(label: &str, first_audio_millis: u64, reconnect: bool) -> Self {
        let root = temp_dir(label);
        let provider_bytes = provider_state();
        let provider_state_sha256 = sha256_hex(&provider_bytes);
        let conversation_bytes = conversation_attempt(&provider_bytes);
        let owner_bytes = snapshot(
            ParticipantRole::Owner,
            1,
            first_audio_millis,
            reconnect,
            &provider_state_sha256,
        );
        let visitor_bytes = snapshot(
            ParticipantRole::Visitor,
            2,
            first_audio_millis,
            reconnect,
            &provider_state_sha256,
        );
        let bound = bind_owner_lab_session_evidence(
            &[owner_bytes.as_slice(), visitor_bytes.as_slice()],
            &provider_bytes,
            CANDIDATE,
        )
        .unwrap();
        let bound_bytes = serde_json::to_vec_pretty(&bound).unwrap();

        let provider = root.join("provider-state.json");
        let conversation = root.join("conversation-attempt.json");
        let bound_path = root.join("bound-session-aggregate.json");
        let owner = root.join("owner-session.json");
        let visitor = root.join("visitor-session.json");
        fs::write(&provider, provider_bytes).unwrap();
        fs::write(&conversation, conversation_bytes).unwrap();
        fs::write(&bound_path, bound_bytes).unwrap();
        fs::write(&owner, owner_bytes).unwrap();
        fs::write(&visitor, visitor_bytes).unwrap();

        Self {
            root,
            provider,
            conversation,
            bound: bound_path,
            owner,
            visitor,
        }
    }

    fn run(&self, candidate: &str) -> std::process::Output {
        self.run_with_snapshots(candidate, &[&self.owner, &self.visitor])
    }

    fn run_with_snapshots(&self, candidate: &str, snapshots: &[&PathBuf]) -> std::process::Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_vpr-rt0-runtime-readiness"));
        command
            .arg(&self.provider)
            .arg(&self.conversation)
            .arg(&self.bound)
            .arg(candidate);
        for snapshot in snapshots {
            command.arg(snapshot);
        }
        command.output().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

fn contains(list: &Value, expected: &str) -> bool {
    list.as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str() == Some(expected))
}

#[test]
fn complete_passing_runtime_evidence_reports_ready_without_claiming_release_ready() {
    let fixture = Fixture::new("passing", 500, true);
    let output = fixture.run(CANDIDATE);
    assert_eq!(output.status.code(), Some(0));
    let report = stdout_json(&output);
    assert_eq!(report["schema_version"], "rt0-runtime-readiness-0.1");
    assert_eq!(report["runtime_supporting_projection"], "passed");
    assert_eq!(report["quality_thresholds"], "passed");
    assert_eq!(report["missing_runtime_evidence"], json!([]));
    assert_eq!(report["release_ready_claimed"], false);
    assert!(contains(
        &report["remaining_non_runtime_requirements"],
        "cost_review"
    ));
    assert!(contains(
        &report["remaining_non_runtime_requirements"],
        "participant_provenance_review"
    ));
    assert!(contains(
        &report["remaining_non_runtime_requirements"],
        "golden_evidence"
    ));
}

#[test]
fn missing_reconnect_is_actionable_and_non_promoting() {
    let fixture = Fixture::new("missing-reconnect", 500, false);
    let output = fixture.run(CANDIDATE);
    assert_eq!(output.status.code(), Some(1));
    let report = stdout_json(&output);
    assert_eq!(report["runtime_supporting_projection"], "failed");
    assert_eq!(report["quality_thresholds"], "failed");
    assert!(contains(
        &report["missing_runtime_evidence"],
        "recoverable_reconnect"
    ));
    assert_eq!(report["release_ready_claimed"], false);
}

#[test]
fn missing_visitor_role_can_never_return_success() {
    let root = temp_dir("owner-only");
    let provider_bytes = provider_state();
    let provider_state_sha256 = sha256_hex(&provider_bytes);
    let conversation_bytes = conversation_attempt(&provider_bytes);
    let owner_bytes = snapshot(ParticipantRole::Owner, 1, 500, true, &provider_state_sha256);
    let bound =
        bind_owner_lab_session_evidence(&[owner_bytes.as_slice()], &provider_bytes, CANDIDATE)
            .unwrap();

    let provider = root.join("provider-state.json");
    let conversation = root.join("conversation-attempt.json");
    let bound_path = root.join("bound-session-aggregate.json");
    let owner = root.join("owner-session.json");
    fs::write(&provider, provider_bytes).unwrap();
    fs::write(&conversation, conversation_bytes).unwrap();
    fs::write(&bound_path, serde_json::to_vec_pretty(&bound).unwrap()).unwrap();
    fs::write(&owner, owner_bytes).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_vpr-rt0-runtime-readiness"))
        .arg(&provider)
        .arg(&conversation)
        .arg(&bound_path)
        .arg(CANDIDATE)
        .arg(&owner)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let report = stdout_json(&output);
    assert!(contains(
        &report["missing_runtime_evidence"],
        "visitor_session_snapshot"
    ));
    assert!(contains(
        &report["missing_runtime_evidence"],
        "visitor_completed_voice_attempt"
    ));
    assert_eq!(report["release_ready_claimed"], false);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn threshold_failure_is_reported_through_the_canonical_quality_contract() {
    let fixture = Fixture::new("audio-threshold", 2000, true);
    let output = fixture.run(CANDIDATE);
    assert_eq!(output.status.code(), Some(1));
    let report = stdout_json(&output);
    assert_eq!(report["runtime_supporting_projection"], "passed");
    assert_eq!(report["quality_thresholds"], "failed");
    assert!(contains(
        &report["quality_threshold_failures"],
        "AUDIO_LATENCY_EXCEEDED"
    ));
    assert_eq!(report["release_ready_claimed"], false);
}

#[test]
fn stale_candidate_binding_fails_closed_without_a_readiness_report() {
    let fixture = Fixture::new("stale-candidate", 500, true);
    let output = fixture.run("2222222222222222222222222222222222222222");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("CONVERSATION_ATTEMPT_INVALID")
    );
}
