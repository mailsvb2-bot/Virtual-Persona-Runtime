mod support;

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

fn exit_evidence(
    golden_bytes: &[u8],
    provider_state_sha256: &str,
    live_provider_probe_sha256: &str,
    conversation_attempt_sha256: &str,
    bound_session_aggregate_sha256: &str,
) -> Value {
    let distribution = |p50, p95| json!({"samples":10,"p50":p50,"p95":p95});
    let conversation = |role: &str, interruption: &str| {
        json!({
            "origin":"real","role":role,"russian":"passed","voice":"passed","video":"passed",
            "completed_turns":2,"interruption_exercised":interruption,"artifact_sha256":digest('a')
        })
    };
    json!({
        "schema_version":"rt0-exit-evidence-0.3",
        "candidate_sha":CANDIDATE,
        "release_spec_sha256":sha256_hex(RELEASE_SPEC),
        "golden_report_sha256":sha256_hex(golden_bytes),
        "provider_state_sha256":provider_state_sha256,
        "live_provider_probe_sha256":live_provider_probe_sha256,
        "conversation_attempt_sha256":conversation_attempt_sha256,
        "bound_session_aggregate_sha256":bound_session_aggregate_sha256,
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

fn live_provider_probe(provider_state_sha256: &str) -> Value {
    let usage = json!({
        "input_units":1,"input_unit":"token","output_units":1,"output_unit":"token",
        "estimated_cost_microunits":1,"provider_charge_microunits":null
    });
    json!({
        "schema_version":"rt0-live-provider-probe-0.1",
        "candidate_sha":CANDIDATE,
        "provider_state_sha256":provider_state_sha256,
        "input_audio_sha256":digest('7'),
        "input_audio_millis":1000,
        "scope":"credentialed_provider_reachability_only",
        "conversation_evidence":false,
        "output_delivery_proven":false,
        "stt":{"latency_millis":100,"transcript_chars":6,"usage":usage.clone()},
        "llm":{"latency_millis":120,"output_chars":5,"usage":usage},
        "avatar":{"open_millis":150,"close_millis":50}
    })
}

fn conversation_attempt(provider_state_sha256: &str) -> Value {
    json!({
        "schema_version":"rt0-live-conversation-attempt-0.1",
        "candidate_sha":CANDIDATE,
        "provider_state_sha256":provider_state_sha256,
        "profile_input_sha256":digest('3'),
        "persona_id_sha256":digest('4'),
        "persona_version":2,
        "reviewed_claims":1,
        "owner":{},
        "visitor":{},
        "conversation_attempted":true,
        "provider_output_submitted":true,
        "browser_media_playback":"not_proven",
        "video_render":"not_proven",
        "human_review":"not_proven"
    })
}

fn bound_session_aggregate(provider_state_sha256: &str) -> Value {
    json!({
        "schema_version":"rt0-owner-lab-session-aggregate-binding-0.1",
        "candidate_sha":CANDIDATE,
        "provider_state_sha256":provider_state_sha256,
        "snapshot_sha256":[digest('9')],
        "aggregate":{
            "schema_version":"rt0-owner-lab-session-aggregate-0.1",
            "source_schema_version":"rt0-owner-lab-session-evidence-0.1",
            "sessions":1,
            "completed_voice_attempts":1,
            "failed_voice_attempts":0,
            "canonical_playback_proven":false,
            "av_sync_proven":false,
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
    })
}


struct PreparedPaths {
    _dir: TempDir,
    evidence: PathBuf,
    golden: PathBuf,
    golden_evidence: PathBuf,
    provider_state: PathBuf,
    live_provider_probe: PathBuf,
    conversation_attempt: PathBuf,
    bound_session_aggregate: PathBuf,
    spec: PathBuf,
}

fn prepare(evidence_mutator: impl FnOnce(&mut Value)) -> PreparedPaths {
    let dir = TempDir::new();
    let golden_path = dir.path().join("golden-report.json");
    let golden_evidence_path = dir.path().join("golden-evidence.json");
    let provider_state_path = dir.path().join("provider-state.json");
    let live_provider_probe_path = dir.path().join("live-provider-probe.json");
    let conversation_attempt_path = dir.path().join("conversation-attempt.json");
    let bound_session_aggregate_path = dir.path().join("bound-session-aggregate.json");
    let evidence_path = dir.path().join("exit-evidence.json");
    let spec_path = dir.path().join("RT0_RELEASE_SPEC.md");
    let fixture = support::fixture(RELEASE_SPEC, CANDIDATE);
    let _ = (&fixture.provider_state, &fixture.bundle);
    let provider_state_bytes = fixture.provider_state_bytes;
    let provider_state_sha256 = sha256_hex(&provider_state_bytes);
    let golden_bytes = serde_json::to_vec_pretty(&fixture.report).unwrap();
    let golden_evidence_bytes = fixture.bundle_bytes;
    let live_provider_probe_bytes =
        serde_json::to_vec_pretty(&live_provider_probe(&provider_state_sha256)).unwrap();
    let conversation_attempt_bytes =
        serde_json::to_vec_pretty(&conversation_attempt(&provider_state_sha256)).unwrap();
    let bound_session_aggregate_bytes =
        serde_json::to_vec_pretty(&bound_session_aggregate(&provider_state_sha256)).unwrap();
    let mut evidence = exit_evidence(
        &golden_bytes,
        &provider_state_sha256,
        &sha256_hex(&live_provider_probe_bytes),
        &sha256_hex(&conversation_attempt_bytes),
        &sha256_hex(&bound_session_aggregate_bytes),
    );
    evidence_mutator(&mut evidence);
    fs::write(&golden_path, golden_bytes).unwrap();
    fs::write(&golden_evidence_path, golden_evidence_bytes).unwrap();
    fs::write(&provider_state_path, provider_state_bytes).unwrap();
    fs::write(&live_provider_probe_path, live_provider_probe_bytes).unwrap();
    fs::write(&conversation_attempt_path, conversation_attempt_bytes).unwrap();
    fs::write(&bound_session_aggregate_path, bound_session_aggregate_bytes).unwrap();
    fs::write(&evidence_path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    fs::write(&spec_path, RELEASE_SPEC).unwrap();
    PreparedPaths {
        _dir: dir,
        evidence: evidence_path,
        golden: golden_path,
        golden_evidence: golden_evidence_path,
        provider_state: provider_state_path,
        live_provider_probe: live_provider_probe_path,
        conversation_attempt: conversation_attempt_path,
        bound_session_aggregate: bound_session_aggregate_path,
        spec: spec_path,
    }
}

fn replace_golden_report_and_rebind_exit_evidence(
    golden: &Path,
    evidence: &Path,
    mutate: impl FnOnce(&mut Value),
) {
    let mut report: Value = serde_json::from_slice(&fs::read(golden).unwrap()).unwrap();
    mutate(&mut report);
    let golden_bytes = serde_json::to_vec_pretty(&report).unwrap();
    fs::write(golden, &golden_bytes).unwrap();

    let mut exit: Value = serde_json::from_slice(&fs::read(evidence).unwrap()).unwrap();
    exit["golden_report_sha256"] = json!(sha256_hex(&golden_bytes));
    fs::write(evidence, serde_json::to_vec_pretty(&exit).unwrap()).unwrap();
}

fn run(paths: &PreparedPaths, candidate: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_vpr-rt0-exit-evidence"))
        .arg(&paths.evidence)
        .arg(&paths.golden)
        .arg(&paths.golden_evidence)
        .arg(&paths.provider_state)
        .arg(&paths.live_provider_probe)
        .arg(&paths.conversation_attempt)
        .arg(&paths.bound_session_aggregate)
        .arg(&paths.spec)
        .arg(candidate)
        .output()
        .unwrap()
}

#[test]
fn cli_returns_zero_only_for_complete_exact_bound_evidence() {
    let paths = prepare(|_| {});
    let output = run(&paths, CANDIDATE);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("\"ready\": true"));
    assert!(!stdout.contains("prompt"));
    assert!(!stdout.contains("response_text"));
}

#[test]
fn cli_returns_one_for_valid_but_privacy_failing_evidence() {
    let paths = prepare(|value| {
            value["privacy_permissions"]["accepted_private_context_leakage"] = json!(1);
        });
    let output = run(&paths, CANDIDATE);
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("PRIVATE_CONTEXT_LEAKAGE_ACCEPTED"));
    assert!(stdout.contains("\"ready\": false"));
}

#[test]
fn cli_rejects_tampered_provider_state_even_when_golden_report_is_valid_json() {
    let paths = prepare(|_| {});
    let mut state: Value = serde_json::from_slice(&fs::read(&paths.provider_state).unwrap()).unwrap();
    state["providers"][0]["provider"] = json!("tampered-stt");
    fs::write(&paths.provider_state, serde_json::to_vec_pretty(&state).unwrap()).unwrap();
    let output = run(&paths, CANDIDATE);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("PROVIDER_STATE_DIGEST_MISMATCH")
    );
}

#[test]
fn cli_rejects_valid_rehashed_probe_from_another_candidate() {
    let paths = prepare(|_| {});
    let mut probe: Value =
        serde_json::from_slice(&fs::read(&paths.live_provider_probe).unwrap()).unwrap();
    probe["candidate_sha"] = json!("2".repeat(40));
    let probe_bytes = serde_json::to_vec_pretty(&probe).unwrap();
    fs::write(&paths.live_provider_probe, &probe_bytes).unwrap();

    let mut exit: Value = serde_json::from_slice(&fs::read(&paths.evidence).unwrap()).unwrap();
    exit["live_provider_probe_sha256"] = json!(sha256_hex(&probe_bytes));
    fs::write(&paths.evidence, serde_json::to_vec_pretty(&exit).unwrap()).unwrap();

    let output = run(&paths, CANDIDATE);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("LIVE_PROVIDER_PROBE_CANDIDATE_MISMATCH")
    );
}

#[test]
fn cli_rejects_rehashed_runtime_evidence_from_wrong_binding() {
    let paths = prepare(|_| {});
    let mut conversation: Value =
        serde_json::from_slice(&fs::read(&paths.conversation_attempt).unwrap()).unwrap();
    conversation["candidate_sha"] = json!("2".repeat(40));
    let conversation_bytes = serde_json::to_vec_pretty(&conversation).unwrap();
    fs::write(&paths.conversation_attempt, &conversation_bytes).unwrap();
    let mut exit: Value = serde_json::from_slice(&fs::read(&paths.evidence).unwrap()).unwrap();
    exit["conversation_attempt_sha256"] = json!(sha256_hex(&conversation_bytes));
    fs::write(&paths.evidence, serde_json::to_vec_pretty(&exit).unwrap()).unwrap();
    let output = run(&paths, CANDIDATE);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("RUNTIME_EVIDENCE_CANDIDATE_MISMATCH")
    );

    let paths = prepare(|_| {});
    let mut session: Value =
        serde_json::from_slice(&fs::read(&paths.bound_session_aggregate).unwrap()).unwrap();
    session["provider_state_sha256"] = json!(digest('8'));
    let session_bytes = serde_json::to_vec_pretty(&session).unwrap();
    fs::write(&paths.bound_session_aggregate, &session_bytes).unwrap();
    let mut exit: Value = serde_json::from_slice(&fs::read(&paths.evidence).unwrap()).unwrap();
    exit["bound_session_aggregate_sha256"] = json!(sha256_hex(&session_bytes));
    fs::write(&paths.evidence, serde_json::to_vec_pretty(&exit).unwrap()).unwrap();
    let output = run(&paths, CANDIDATE);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("RUNTIME_EVIDENCE_PROVIDER_STATE_MISMATCH")
    );
}

#[test]
fn cli_returns_two_for_stale_candidate_or_unknown_fields() {
    let paths = prepare(|_| {});
    let output = run(&paths, &"2".repeat(40));
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("CANDIDATE_SHA_MISMATCH")
    );

    let paths = prepare(|value| {
            value["api_key"] = json!("must-not-be-accepted");
        });
    let output = run(&paths, CANDIDATE);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("INPUT_INVALID")
    );
}

#[test]
fn cli_rejects_forged_shortened_golden_report_after_rebinding_hash() {
    let paths = prepare(|_| {});
    replace_golden_report_and_rebind_exit_evidence(&paths.golden, &paths.evidence, |report| {
        report["golden"]["total"] = json!(1);
        report["golden"]["passed"] = json!(1);
        report["golden"]["failed"] = json!(0);
        report["golden"]["cases"] = json!([report["golden"]["cases"][0].clone()]);
    });
    let output = run(&paths, CANDIDATE);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("GOLDEN_REPORT_RECOMPUTE_MISMATCH")
    );
}

#[test]
fn cli_rejects_overflowing_golden_counts_without_panicking() {
    let paths = prepare(|_| {});
    replace_golden_report_and_rebind_exit_evidence(&paths.golden, &paths.evidence, |report| {
        report["golden"]["passed"] = json!(usize::MAX);
        report["golden"]["failed"] = json!(1);
    });
    let output = run(&paths, CANDIDATE);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("GOLDEN_REPORT_INVALID")
    );
}
