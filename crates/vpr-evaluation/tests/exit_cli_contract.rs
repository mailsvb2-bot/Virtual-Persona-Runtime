mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use vpr_evaluation::{bind_owner_lab_session_evidence, sha256_hex};

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
    supporting_artifact_sha256: &str,
) -> Value {
    let distribution = |p50, p95| json!({"samples":10,"p50":p50,"p95":p95});
    let conversation = |role: &str, interruption: &str| {
        json!({
            "origin":"real","role":role,"russian":"passed","voice":"passed","video":"passed",
            "completed_turns":2,"interruption_exercised":interruption,"artifact_sha256":supporting_artifact_sha256
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
            "ci":{"status":"passed","artifact_sha256":supporting_artifact_sha256},
            "e2e":{"status":"passed","artifact_sha256":supporting_artifact_sha256}
        },
        "conversations":{
            "owner":conversation("owner","passed"),
            "visitor":conversation("visitor","failed")
        },
        "acceptance":{
            "origin":"real","owner_happy_path":"passed","visitor_happy_path":"passed",
            "correction_path":"passed","failure_recovery_path":"passed","revoke_deny_path":"passed",
            "artifact_sha256":supporting_artifact_sha256
        },
        "quality":{
            "origin":"real",
            "text_first_meaningful_response":distribution(900,2400),
            "first_meaningful_audio":distribution(1400,2900),
            "interruption_stop":distribution(250,450),
            "first_useful_video":distribution(1200,2400),
            "av_sync_absolute_offset":{"samples":3,"p50":50,"p95":110},
            "recoverable_reconnect":distribution(2000,4900),
            "artifact_sha256":supporting_artifact_sha256
        },
        "cost":{
            "origin":"real","measured_duration_millis":60000,"measured_cost_microunits":1234,
            "provider_charge_microunits":null,"artifact_sha256":supporting_artifact_sha256
        },
        "privacy_permissions":{
            "origin":"real","permission_suite":"passed","accepted_private_context_leakage":0,
            "accepted_false_owner_attribution":0,"revocation":"passed","egress_denial":"passed",
            "artifact_sha256":supporting_artifact_sha256
        },
        "human_evaluation":{
            "origin":"real","rubric_version":"rt0-human-v1","reviewer_count":1,
            "dimensions":{"voice_similarity":"recorded","voice_naturalness":"recorded",
                "appearance_plausibility":"recorded","persona_similarity":"recorded",
                "conversation_naturalness":"recorded"},
            "usable_for_continuation":"passed","artifact_sha256":supporting_artifact_sha256
        },
        "known_limitations":{"review_status":"passed","document_sha256":supporting_artifact_sha256}
    })
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

fn bind_json_claim(root: &Path, name: &str, claim: &mut Value) {
    let bytes = claim_bytes(claim);
    fs::write(root.join(name), &bytes).unwrap();
    claim["artifact_sha256"] = json!(sha256_hex(&bytes));
}

fn bind_automated_claim(root: &Path, name: &str, candidate_sha: &str, claim: &mut Value) {
    let mut projected: Value = serde_json::from_slice(&claim_bytes(claim)).unwrap();
    projected["candidate_sha"] = json!(candidate_sha);
    let bytes = serde_json::to_vec_pretty(&projected).unwrap();
    fs::write(root.join(name), &bytes).unwrap();
    claim["artifact_sha256"] = json!(sha256_hex(&bytes));
}

fn bind_real_claim(
    root: &Path,
    name: &str,
    candidate_sha: &str,
    provider_state_sha256: &str,
    claim: &mut Value,
) {
    let mut projected: Value = serde_json::from_slice(&claim_bytes(claim)).unwrap();
    projected["candidate_sha"] = json!(candidate_sha);
    projected["provider_state_sha256"] = json!(provider_state_sha256);
    let bytes = serde_json::to_vec_pretty(&projected).unwrap();
    fs::write(root.join(name), &bytes).unwrap();
    claim["artifact_sha256"] = json!(sha256_hex(&bytes));
}

fn bind_supporting_artifacts(root: &Path, evidence: &mut Value) {
    let candidate_sha = evidence["candidate_sha"].as_str().unwrap().to_owned();
    let provider_state_sha256 = evidence["provider_state_sha256"]
        .as_str()
        .unwrap()
        .to_owned();
    bind_automated_claim(
        root,
        "ci-evidence.json",
        &candidate_sha,
        &mut evidence["automated"]["ci"],
    );
    bind_automated_claim(
        root,
        "e2e-evidence.json",
        &candidate_sha,
        &mut evidence["automated"]["e2e"],
    );
    bind_real_claim(
        root,
        "owner-conversation.json",
        &candidate_sha,
        &provider_state_sha256,
        &mut evidence["conversations"]["owner"],
    );
    bind_real_claim(
        root,
        "visitor-conversation.json",
        &candidate_sha,
        &provider_state_sha256,
        &mut evidence["conversations"]["visitor"],
    );
    bind_real_claim(
        root,
        "acceptance.json",
        &candidate_sha,
        &provider_state_sha256,
        &mut evidence["acceptance"],
    );
    bind_real_claim(
        root,
        "quality.json",
        &candidate_sha,
        &provider_state_sha256,
        &mut evidence["quality"],
    );
    bind_real_claim(
        root,
        "cost.json",
        &candidate_sha,
        &provider_state_sha256,
        &mut evidence["cost"],
    );
    bind_real_claim(
        root,
        "privacy-permissions.json",
        &candidate_sha,
        &provider_state_sha256,
        &mut evidence["privacy_permissions"],
    );
    bind_real_claim(
        root,
        "human-evaluation.json",
        &candidate_sha,
        &provider_state_sha256,
        &mut evidence["human_evaluation"],
    );
    let limitations = b"reviewed RT0 limitations\n";
    fs::write(root.join("known-limitations.md"), limitations).unwrap();
    evidence["known_limitations"]["document_sha256"] = json!(sha256_hex(limitations));
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

fn session_snapshot() -> Value {
    json!({
        "schema_version":"rt0-owner-lab-session-evidence-0.3",
        "scope":"browser_observed_media_plane_only",
        "session_sequence":1,
        "canonical_playback_proven":true,
        "av_sync_proven":true,
        "voice_attempts":[{
            "request_sequence":1,
            "canonical_turn_sequence":11,
            "canonical_output_sequence":12,
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
        "media_events":[{"request_sequence":1,"kind":"audio_started","elapsed_millis":500}],
        "av_sync_samples":[
            {"request_sequence":1,"sample_sequence":1,"reference":"web_rtc_estimated_playout_timestamp","absolute_offset_millis":40},
            {"request_sequence":1,"sample_sequence":2,"reference":"web_rtc_estimated_playout_timestamp","absolute_offset_millis":50},
            {"request_sequence":1,"sample_sequence":3,"reference":"web_rtc_estimated_playout_timestamp","absolute_offset_millis":110}
        ]
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
    supporting_artifacts: PathBuf,
    session_snapshot: PathBuf,
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
    let supporting_artifacts_path = dir.path().join("supporting");
    let session_snapshot_path = dir.path().join("session-snapshot.json");
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
    let session_snapshot_bytes = serde_json::to_vec_pretty(&session_snapshot()).unwrap();
    let bound_session_aggregate = bind_owner_lab_session_evidence(
        &[session_snapshot_bytes.as_slice()],
        &provider_state_bytes,
        CANDIDATE,
    )
    .unwrap();
    let bound_session_aggregate_bytes =
        serde_json::to_vec_pretty(&bound_session_aggregate).unwrap();
    fs::create_dir(&supporting_artifacts_path).unwrap();
    let mut evidence = exit_evidence(
        &golden_bytes,
        &provider_state_sha256,
        &sha256_hex(&live_provider_probe_bytes),
        &sha256_hex(&conversation_attempt_bytes),
        &sha256_hex(&bound_session_aggregate_bytes),
        &digest('0'),
    );
    evidence_mutator(&mut evidence);
    bind_supporting_artifacts(&supporting_artifacts_path, &mut evidence);
    fs::write(&golden_path, golden_bytes).unwrap();
    fs::write(&golden_evidence_path, golden_evidence_bytes).unwrap();
    fs::write(&provider_state_path, provider_state_bytes).unwrap();
    fs::write(&live_provider_probe_path, live_provider_probe_bytes).unwrap();
    fs::write(&conversation_attempt_path, conversation_attempt_bytes).unwrap();
    fs::write(&bound_session_aggregate_path, bound_session_aggregate_bytes).unwrap();
    fs::write(&session_snapshot_path, session_snapshot_bytes).unwrap();
    fs::write(
        &evidence_path,
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
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
        supporting_artifacts: supporting_artifacts_path,
        session_snapshot: session_snapshot_path,
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
        .arg(&paths.supporting_artifacts)
        .arg(&paths.session_snapshot)
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
    let mut state: Value =
        serde_json::from_slice(&fs::read(&paths.provider_state).unwrap()).unwrap();
    state["providers"][0]["provider"] = json!("tampered-stt");
    fs::write(
        &paths.provider_state,
        serde_json::to_vec_pretty(&state).unwrap(),
    )
    .unwrap();
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
fn cli_rejects_raw_snapshot_that_does_not_recompute_bound_aggregate() {
    let paths = prepare(|_| {});
    let mut snapshot: Value =
        serde_json::from_slice(&fs::read(&paths.session_snapshot).unwrap()).unwrap();
    snapshot["media_events"][0]["elapsed_millis"] = json!(501);
    fs::write(
        &paths.session_snapshot,
        serde_json::to_vec_pretty(&snapshot).unwrap(),
    )
    .unwrap();

    let output = run(&paths, CANDIDATE);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("RUNTIME_EVIDENCE_INVALID")
    );
}

#[test]
fn cli_rejects_tampered_supporting_artifact_with_well_formed_declared_digest() {
    let paths = prepare(|_| {});
    fs::write(
        paths.supporting_artifacts.join("ci-evidence.json"),
        b"tampered ci evidence\n",
    )
    .unwrap();
    let output = run(&paths, CANDIDATE);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("INVALID_ARTIFACT_DIGEST")
    );
}

#[test]
fn cli_rejects_rehashed_supporting_artifact_with_detached_claim() {
    let paths = prepare(|_| {});
    let quality_path = paths.supporting_artifacts.join("quality.json");
    let mut quality: Value = serde_json::from_slice(&fs::read(&quality_path).unwrap()).unwrap();
    quality["recoverable_reconnect"]["p95"] = json!(1);
    let quality_bytes = serde_json::to_vec_pretty(&quality).unwrap();
    fs::write(&quality_path, &quality_bytes).unwrap();

    let mut evidence: Value = serde_json::from_slice(&fs::read(&paths.evidence).unwrap()).unwrap();
    evidence["quality"]["artifact_sha256"] = json!(sha256_hex(&quality_bytes));
    fs::write(
        &paths.evidence,
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();

    let output = run(&paths, CANDIDATE);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("INVALID_ARTIFACT_DIGEST")
    );
}

#[test]
fn cli_rejects_rehashed_automated_artifact_from_another_candidate() {
    let paths = prepare(|_| {});
    let ci_path = paths.supporting_artifacts.join("ci-evidence.json");
    let mut ci: Value = serde_json::from_slice(&fs::read(&ci_path).unwrap()).unwrap();
    ci["candidate_sha"] = json!("2".repeat(40));
    let ci_bytes = serde_json::to_vec_pretty(&ci).unwrap();
    fs::write(&ci_path, &ci_bytes).unwrap();

    let mut evidence: Value = serde_json::from_slice(&fs::read(&paths.evidence).unwrap()).unwrap();
    evidence["automated"]["ci"]["artifact_sha256"] = json!(sha256_hex(&ci_bytes));
    fs::write(
        &paths.evidence,
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();

    let output = run(&paths, CANDIDATE);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("INVALID_ARTIFACT_DIGEST")
    );
}

#[test]
fn cli_rejects_rehashed_real_artifact_from_another_provider_state() {
    let paths = prepare(|_| {});
    let quality_path = paths.supporting_artifacts.join("quality.json");
    let mut quality: Value = serde_json::from_slice(&fs::read(&quality_path).unwrap()).unwrap();
    quality["provider_state_sha256"] = json!("2".repeat(64));
    let quality_bytes = serde_json::to_vec_pretty(&quality).unwrap();
    fs::write(&quality_path, &quality_bytes).unwrap();

    let mut evidence: Value = serde_json::from_slice(&fs::read(&paths.evidence).unwrap()).unwrap();
    evidence["quality"]["artifact_sha256"] = json!(sha256_hex(&quality_bytes));
    fs::write(
        &paths.evidence,
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();

    let output = run(&paths, CANDIDATE);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("INVALID_ARTIFACT_DIGEST")
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
