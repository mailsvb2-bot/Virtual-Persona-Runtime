mod support;

use serde_json::{Value, json};
use vpr_evaluation::{
    CheckStatus, Rt0ExitAssemblyError, Rt0ExitAssemblyInputs, Rt0SupportingPreflightArtifacts,
    assemble_rt0_exit_evidence, bind_owner_lab_session_evidence, sha256_hex,
};

const CANDIDATE: &str = "1111111111111111111111111111111111111111";
const RELEASE_SPEC: &[u8] = b"rt0 assembler release spec";

struct SupportingFixture {
    ci: Vec<u8>,
    e2e: Vec<u8>,
    owner: Vec<u8>,
    visitor: Vec<u8>,
    acceptance: Vec<u8>,
    quality: Vec<u8>,
    cost: Vec<u8>,
    privacy: Vec<u8>,
    human: Vec<u8>,
    limitations: Vec<u8>,
}

impl SupportingFixture {
    fn new(provider_state_sha256: &str) -> Self {
        let text_distribution = json!({"samples": 1, "p50": 100, "p95": 100});
        Self {
            ci: bytes(&json!({"status":"failed","candidate_sha":CANDIDATE})),
            e2e: bytes(&json!({"status":"passed","candidate_sha":CANDIDATE})),
            owner: bytes(&json!({
                "origin":"real",
                "role":"owner",
                "russian":"passed",
                "voice":"passed",
                "video":"passed",
                "completed_turns":1,
                "interruption_exercised":"passed",
                "candidate_sha":CANDIDATE,
                "provider_state_sha256":provider_state_sha256
            })),
            visitor: bytes(&json!({
                "origin":"real",
                "role":"visitor",
                "russian":"passed",
                "voice":"passed",
                "video":"passed",
                "completed_turns":1,
                "interruption_exercised":"failed",
                "candidate_sha":CANDIDATE,
                "provider_state_sha256":provider_state_sha256
            })),
            acceptance: bytes(&json!({
                "origin":"real",
                "owner_happy_path":"passed",
                "visitor_happy_path":"passed",
                "correction_path":"passed",
                "failure_recovery_path":"failed",
                "revoke_deny_path":"passed",
                "candidate_sha":CANDIDATE,
                "provider_state_sha256":provider_state_sha256
            })),
            quality: bytes(&json!({
                "origin":"real",
                "text_first_meaningful_response":text_distribution,
                "first_meaningful_audio":{"samples":1,"p50":500,"p95":500},
                "interruption_stop":{"samples":1,"p50":250,"p95":250},
                "first_useful_video":{"samples":1,"p50":700,"p95":700},
                "av_sync_absolute_offset":{"samples":3,"p50":60,"p95":120},
                "recoverable_reconnect":{"samples":1,"p50":800,"p95":800},
                "candidate_sha":CANDIDATE,
                "provider_state_sha256":provider_state_sha256
            })),
            cost: bytes(&json!({
                "origin":"real",
                "measured_duration_millis":60000,
                "measured_cost_microunits":null,
                "provider_charge_microunits":null,
                "candidate_sha":CANDIDATE,
                "provider_state_sha256":provider_state_sha256
            })),
            privacy: bytes(&json!({
                "origin":"real",
                "permission_suite":"failed",
                "accepted_private_context_leakage":1,
                "accepted_false_owner_attribution":0,
                "revocation":"passed",
                "egress_denial":"passed",
                "candidate_sha":CANDIDATE,
                "provider_state_sha256":provider_state_sha256
            })),
            human: bytes(&json!({
                "origin":"real",
                "rubric_version":"rt0-human-rubric-1",
                "reviewer_count":1,
                "dimensions":{
                    "voice_similarity":"recorded",
                    "voice_naturalness":"recorded",
                    "appearance_plausibility":"recorded",
                    "persona_similarity":"recorded",
                    "conversation_naturalness":"recorded"
                },
                "usable_for_continuation":"failed",
                "candidate_sha":CANDIDATE,
                "provider_state_sha256":provider_state_sha256
            })),
            limitations:
                b"RT0-Review-Status: failed\n\nKnown limitation recorded by the reviewer.\n"
                    .to_vec(),
        }
    }

    fn as_preflight(&self) -> Rt0SupportingPreflightArtifacts<'_> {
        Rt0SupportingPreflightArtifacts {
            ci: &self.ci,
            e2e: &self.e2e,
            owner_conversation: &self.owner,
            visitor_conversation: &self.visitor,
            acceptance: &self.acceptance,
            quality: &self.quality,
            cost: &self.cost,
            privacy_permissions: &self.privacy,
            human_evaluation: &self.human,
            known_limitations: &self.limitations,
        }
    }
}

struct AssemblyFixture {
    golden: Vec<u8>,
    provider_state: Vec<u8>,
    probe: Vec<u8>,
    conversation: Vec<u8>,
    session: Vec<u8>,
    supporting: SupportingFixture,
}

impl AssemblyFixture {
    fn new() -> Self {
        let golden_fixture = support::fixture(RELEASE_SPEC, CANDIDATE);
        assert_eq!(golden_fixture.provider_state.providers.len(), 3);
        assert_eq!(golden_fixture.bundle.binding.candidate_sha, CANDIDATE);
        assert!(!golden_fixture.bundle_bytes.is_empty());

        let provider_state = golden_fixture.provider_state_bytes;
        let provider_digest = sha256_hex(&provider_state);
        let golden = serde_json::to_vec_pretty(&golden_fixture.report).unwrap();
        let probe = probe_bytes(&provider_digest);
        let conversation = conversation_bytes(&provider_digest);
        let session = session_bytes(&provider_state);
        let supporting = SupportingFixture::new(&provider_digest);
        Self {
            golden,
            provider_state,
            probe,
            conversation,
            session,
            supporting,
        }
    }

    fn inputs(&self) -> Rt0ExitAssemblyInputs<'_> {
        Rt0ExitAssemblyInputs {
            supporting: self.supporting.as_preflight(),
            bound_golden_report_bytes: &self.golden,
            provider_state_bytes: &self.provider_state,
            live_provider_probe_bytes: &self.probe,
            conversation_attempt_bytes: &self.conversation,
            bound_session_aggregate_bytes: &self.session,
            release_spec_bytes: RELEASE_SPEC,
            exact_candidate_sha: CANDIDATE,
        }
    }
}

fn bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec_pretty(value).unwrap()
}

fn probe_bytes(provider_digest: &str) -> Vec<u8> {
    let usage = json!({
        "input_units":1,
        "input_unit":"token",
        "output_units":1,
        "output_unit":"token",
        "estimated_cost_microunits":1,
        "provider_charge_microunits":null
    });
    bytes(&json!({
        "schema_version":"rt0-live-provider-probe-0.3",
        "candidate_sha":CANDIDATE,
        "provider_state_sha256":provider_digest,
        "input_audio_sha256":"7".repeat(64),
        "input_audio_millis":1000,
        "scope":"credentialed_provider_reachability_only",
        "conversation_evidence":false,
        "output_delivery_proven":false,
        "stt":{"latency_millis":100,"transcript_chars":6,"usage":usage.clone()},
        "llm":{"latency_millis":120,"output_chars":5,"usage":usage.clone()},
        "avatar":{"open_millis":150,"close_millis":50}
    }))
}

fn conversation_bytes(provider_digest: &str) -> Vec<u8> {
    bytes(&json!({
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
        "provider_output_submitted":true
    }))
}

fn session_bytes(provider_state: &[u8]) -> Vec<u8> {
    let snapshot = bytes(&json!({
        "schema_version":"rt0-owner-lab-session-evidence-0.6",
        "scope":"browser_observed_media_plane_only",
        "session_sequence":1,
        "participant_role":"owner",
        "canonical_playback_proven":true,
        "av_sync_proven":true,
        "text_attempts":[{
            "request_sequence":1,
            "canonical_turn_sequence":6,
            "canonical_output_sequence":16,
            "status":"completed",
            "failure_code":null,
            "first_meaningful_response_millis":100,
            "server_total_millis":150,
            "llm_usage":{
                "input_units":1,
                "output_units":1,
                "estimated_cost_microunits":1,
                "provider_charge_microunits":null
            }
        }],
        "voice_attempts":[{
            "request_sequence":1,
            "canonical_turn_sequence":11,
            "canonical_output_sequence":21,
            "canonical_playback_confirmed":true,
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
        }],
        "media_events":[
            {"request_sequence":1,"kind":"audio_started","elapsed_millis":500},
            {"request_sequence":null,"kind":"video_ready","elapsed_millis":700},
            {"request_sequence":1,"kind":"interruption_stopped","elapsed_millis":250},
            {"request_sequence":null,"kind":"reconnect_restored","elapsed_millis":800}
        ],
        "av_sync_samples":[
            {"request_sequence":1,"sample_sequence":1,"reference":"web_rtc_estimated_playout_timestamp","absolute_offset_millis":40},
            {"request_sequence":1,"sample_sequence":2,"reference":"web_rtc_estimated_playout_timestamp","absolute_offset_millis":60},
            {"request_sequence":1,"sample_sequence":3,"reference":"web_rtc_estimated_playout_timestamp","absolute_offset_millis":120}
        ]
    }));
    let bound =
        bind_owner_lab_session_evidence(&[snapshot.as_slice()], provider_state, CANDIDATE).unwrap();
    serde_json::to_vec_pretty(&bound).unwrap()
}

#[test]
fn assembler_preserves_failed_real_evidence_without_claiming_readiness() {
    let fixture = AssemblyFixture::new();
    let evidence = assemble_rt0_exit_evidence(fixture.inputs()).unwrap();

    assert_eq!(evidence.automated.ci.status, CheckStatus::Failed);
    assert_eq!(
        evidence.acceptance.failure_recovery_path,
        CheckStatus::Failed
    );
    assert_eq!(
        evidence
            .privacy_permissions
            .accepted_private_context_leakage,
        1
    );
    assert_eq!(
        evidence.human_evaluation.usable_for_continuation,
        CheckStatus::Failed
    );
    assert_eq!(
        evidence.known_limitations.review_status,
        CheckStatus::Failed
    );
    assert_eq!(
        evidence.automated.ci.artifact_sha256,
        sha256_hex(&fixture.supporting.ci)
    );
    let serialized = serde_json::to_value(&evidence).unwrap();
    assert!(serialized.get("ready").is_none());
}

#[test]
fn assembler_rejects_browser_quality_detached_from_bound_session() {
    let mut fixture = AssemblyFixture::new();
    let mut quality: Value = serde_json::from_slice(&fixture.supporting.quality).unwrap();
    quality["recoverable_reconnect"]["p50"] = json!(700);
    quality["recoverable_reconnect"]["p95"] = json!(700);
    fixture.supporting.quality = bytes(&quality);

    assert_eq!(
        assemble_rt0_exit_evidence(fixture.inputs()),
        Err(Rt0ExitAssemblyError::BrowserQualityMismatch)
    );
}

#[test]
fn stale_golden_candidate_is_rejected() {
    let mut fixture = AssemblyFixture::new();
    let mut golden: Value = serde_json::from_slice(&fixture.golden).unwrap();
    golden["binding"]["candidate_sha"] = json!("2".repeat(40));
    fixture.golden = bytes(&golden);

    assert_eq!(
        assemble_rt0_exit_evidence(fixture.inputs()),
        Err(Rt0ExitAssemblyError::GoldenCandidateMismatch)
    );
}

#[test]
fn stale_probe_conversation_and_session_bindings_are_rejected() {
    let mut fixture = AssemblyFixture::new();
    let mut probe: Value = serde_json::from_slice(&fixture.probe).unwrap();
    probe["candidate_sha"] = json!("2".repeat(40));
    fixture.probe = bytes(&probe);
    assert_eq!(
        assemble_rt0_exit_evidence(fixture.inputs()),
        Err(Rt0ExitAssemblyError::LiveProviderProbeInvalid)
    );

    let mut fixture = AssemblyFixture::new();
    let mut conversation: Value = serde_json::from_slice(&fixture.conversation).unwrap();
    conversation["candidate_sha"] = json!("2".repeat(40));
    fixture.conversation = bytes(&conversation);
    assert_eq!(
        assemble_rt0_exit_evidence(fixture.inputs()),
        Err(Rt0ExitAssemblyError::ConversationCandidateMismatch)
    );

    let mut fixture = AssemblyFixture::new();
    let mut session: Value = serde_json::from_slice(&fixture.session).unwrap();
    session["provider_state_sha256"] = json!("2".repeat(64));
    fixture.session = bytes(&session);
    assert_eq!(
        assemble_rt0_exit_evidence(fixture.inputs()),
        Err(Rt0ExitAssemblyError::SessionProviderStateMismatch)
    );
}

#[test]
fn supporting_binding_failure_stops_assembly() {
    let mut fixture = AssemblyFixture::new();
    let mut privacy: Value = serde_json::from_slice(&fixture.supporting.privacy).unwrap();
    privacy["provider_state_sha256"] = json!("2".repeat(64));
    fixture.supporting.privacy = bytes(&privacy);

    assert_eq!(
        assemble_rt0_exit_evidence(fixture.inputs()),
        Err(Rt0ExitAssemblyError::SupportingEvidenceInvalid)
    );
}
