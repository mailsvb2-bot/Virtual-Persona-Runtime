mod support;

use vpr_evaluation::{
    AcceptanceEvidence, ArtifactCheckEvidence, AutomatedEvidence, AvatarProbeEvidence,
    BoundGoldenReport, BoundLabSessionEvidenceAggregate, CheckStatus, ConversationEvidence,
    ConversationPairEvidence, CostEvidence, EvidenceOrigin, EvidenceVerificationContext,
    GoldenEvidenceBundle, GoldenSuite, HumanDimensions, HumanEvaluationEvidence,
    KnownLimitationsEvidence, LatencyDistributionMillis, LiveProviderProbeReceipt,
    LlmProbeEvidence, ParticipantRole, PrivacyPermissionEvidence, ProbeUsage, QualityEvidence,
    RT0_EXIT_EVIDENCE_SCHEMA, RT0_LIVE_PROVIDER_PROBE_SCHEMA, RecordStatus, Rt0ExitEvidence,
    Rt0ExitEvidenceError, Rt0ExitFailureCode, Rt0ExitVerificationContext, SttProbeEvidence,
    bind_owner_lab_session_evidence, evaluate_bound_golden_suite, evaluate_rt0_exit_evidence,
    sha256_hex,
};

const CANDIDATE: &str = "1111111111111111111111111111111111111111";
const RELEASE_SPEC: &[u8] = b"rt0 release spec contract";

fn digest(byte: char) -> String {
    byte.to_string().repeat(64)
}

fn golden_fixture() -> support::GoldenFixture {
    support::fixture(RELEASE_SPEC, CANDIDATE)
}

fn failing_golden_fixture() -> support::GoldenFixture {
    let fixture = golden_fixture();
    let mut value: serde_json::Value = serde_json::from_slice(&fixture.bundle_bytes).unwrap();
    value["observations"][0]["response_text"] =
        serde_json::json!("не содержит обязательного имени");
    let bundle_bytes = serde_json::to_vec_pretty(&value).unwrap();
    let bundle: GoldenEvidenceBundle = serde_json::from_slice(&bundle_bytes).unwrap();
    let suite: GoldenSuite = serde_json::from_slice(support::SUITE_BYTES).unwrap();
    let report = evaluate_bound_golden_suite(
        &suite,
        &bundle,
        EvidenceVerificationContext {
            suite_bytes: support::SUITE_BYTES,
            release_spec_bytes: RELEASE_SPEC,
            provider_state: &fixture.provider_state,
            provider_state_bytes: &fixture.provider_state_bytes,
            evidence_bytes: &bundle_bytes,
            exact_candidate_sha: CANDIDATE,
        },
    )
    .unwrap();
    support::GoldenFixture {
        provider_state: fixture.provider_state,
        provider_state_bytes: fixture.provider_state_bytes,
        bundle,
        bundle_bytes,
        report,
    }
}

fn check() -> ArtifactCheckEvidence {
    ArtifactCheckEvidence {
        status: CheckStatus::Passed,
        artifact_sha256: digest('d'),
    }
}

fn conversation(role: ParticipantRole) -> ConversationEvidence {
    ConversationEvidence {
        origin: EvidenceOrigin::Real,
        role,
        russian: CheckStatus::Passed,
        voice: CheckStatus::Passed,
        video: CheckStatus::Passed,
        completed_turns: 2,
        interruption_exercised: if role == ParticipantRole::Owner {
            CheckStatus::Passed
        } else {
            CheckStatus::Failed
        },
        artifact_sha256: digest('f'),
    }
}

fn distribution(p50: u64, p95: u64) -> LatencyDistributionMillis {
    LatencyDistributionMillis {
        samples: 20,
        p50,
        p95,
    }
}

fn probe_usage() -> ProbeUsage {
    ProbeUsage {
        input_units: Some(1),
        input_unit: Some("token".into()),
        output_units: Some(1),
        output_unit: Some("token".into()),
        estimated_cost_microunits: Some(1),
        provider_charge_microunits: None,
    }
}

fn live_provider_probe(provider_state_bytes: &[u8]) -> LiveProviderProbeReceipt {
    LiveProviderProbeReceipt {
        schema_version: RT0_LIVE_PROVIDER_PROBE_SCHEMA.into(),
        candidate_sha: CANDIDATE.into(),
        provider_state_sha256: sha256_hex(provider_state_bytes),
        input_audio_sha256: digest('7'),
        input_audio_millis: 1_000,
        scope: "credentialed_provider_reachability_only".into(),
        conversation_evidence: false,
        output_delivery_proven: false,
        stt: SttProbeEvidence {
            latency_millis: 100,
            transcript_chars: 6,
            usage: probe_usage(),
        },
        llm: LlmProbeEvidence {
            latency_millis: 120,
            output_chars: 5,
            usage: probe_usage(),
        },
        avatar: AvatarProbeEvidence {
            open_millis: 150,
            close_millis: 50,
        },
    }
}

fn live_provider_probe_bytes(provider_state_bytes: &[u8]) -> Vec<u8> {
    serde_json::to_vec(&live_provider_probe(provider_state_bytes)).unwrap()
}

fn conversation_attempt_bytes(provider_state_bytes: &[u8]) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version":"rt0-live-conversation-attempt-0.1",
        "candidate_sha":CANDIDATE,
        "provider_state_sha256":sha256_hex(provider_state_bytes),
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
    }))
    .unwrap()
}

fn session_snapshot_bytes_with_av_sync(offsets: [u64; 3]) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
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
            "stt_usage":{
                "input_units":1,"output_units":0,
                "estimated_cost_microunits":1,"provider_charge_microunits":null
            },
            "llm_usage":{
                "input_units":1,"output_units":1,
                "estimated_cost_microunits":1,"provider_charge_microunits":null
            }
        }],
        "media_events":[{
            "request_sequence":1,
            "kind":"audio_started",
            "elapsed_millis":500
        }],
        "av_sync_samples":[
            {"request_sequence":1,"sample_sequence":1,"reference":"web_rtc_estimated_playout_timestamp","absolute_offset_millis":offsets[0]},
            {"request_sequence":1,"sample_sequence":2,"reference":"web_rtc_estimated_playout_timestamp","absolute_offset_millis":offsets[1]},
            {"request_sequence":1,"sample_sequence":3,"reference":"web_rtc_estimated_playout_timestamp","absolute_offset_millis":offsets[2]}
        ]
    }))
    .unwrap()
}

fn session_snapshot_bytes() -> Vec<u8> {
    session_snapshot_bytes_with_av_sync([40, 60, 120])
}

fn bound_session_aggregate(provider_state_bytes: &[u8]) -> BoundLabSessionEvidenceAggregate {
    let snapshot = session_snapshot_bytes();
    bind_owner_lab_session_evidence(&[snapshot.as_slice()], provider_state_bytes, CANDIDATE)
        .unwrap()
}

fn passing_evidence(golden_bytes: &[u8], provider_state_bytes: &[u8]) -> Rt0ExitEvidence {
    Rt0ExitEvidence {
        schema_version: RT0_EXIT_EVIDENCE_SCHEMA.into(),
        candidate_sha: CANDIDATE.into(),
        release_spec_sha256: sha256_hex(RELEASE_SPEC),
        golden_report_sha256: sha256_hex(golden_bytes),
        provider_state_sha256: sha256_hex(provider_state_bytes),
        live_provider_probe_sha256: sha256_hex(&live_provider_probe_bytes(provider_state_bytes)),
        conversation_attempt_sha256: sha256_hex(&conversation_attempt_bytes(provider_state_bytes)),
        bound_session_aggregate_sha256: sha256_hex(
            &serde_json::to_vec(&bound_session_aggregate(provider_state_bytes)).unwrap(),
        ),
        automated: AutomatedEvidence {
            ci: check(),
            e2e: check(),
        },
        conversations: ConversationPairEvidence {
            owner: conversation(ParticipantRole::Owner),
            visitor: conversation(ParticipantRole::Visitor),
        },
        acceptance: AcceptanceEvidence {
            origin: EvidenceOrigin::Real,
            owner_happy_path: CheckStatus::Passed,
            visitor_happy_path: CheckStatus::Passed,
            correction_path: CheckStatus::Passed,
            failure_recovery_path: CheckStatus::Passed,
            revoke_deny_path: CheckStatus::Passed,
            artifact_sha256: digest('1'),
        },
        quality: QualityEvidence {
            origin: EvidenceOrigin::Real,
            text_first_meaningful_response: distribution(1_000, 2_500),
            first_meaningful_audio: distribution(1_500, 3_000),
            interruption_stop: distribution(250, 500),
            first_useful_video: distribution(1_250, 2_500),
            av_sync_absolute_offset: LatencyDistributionMillis {
                samples: 3,
                p50: 60,
                p95: 120,
            },
            recoverable_reconnect: distribution(2_500, 5_000),
            artifact_sha256: digest('2'),
        },
        cost: CostEvidence {
            origin: EvidenceOrigin::Real,
            measured_duration_millis: 30_000,
            measured_cost_microunits: Some(3_000),
            provider_charge_microunits: None,
            artifact_sha256: digest('3'),
        },
        privacy_permissions: PrivacyPermissionEvidence {
            origin: EvidenceOrigin::Real,
            permission_suite: CheckStatus::Passed,
            accepted_private_context_leakage: 0,
            accepted_false_owner_attribution: 0,
            revocation: CheckStatus::Passed,
            egress_denial: CheckStatus::Passed,
            artifact_sha256: digest('4'),
        },
        human_evaluation: HumanEvaluationEvidence {
            origin: EvidenceOrigin::Real,
            rubric_version: "rt0-human-contract-v1".into(),
            reviewer_count: 2,
            dimensions: HumanDimensions {
                voice_similarity: RecordStatus::Recorded,
                voice_naturalness: RecordStatus::Recorded,
                appearance_plausibility: RecordStatus::Recorded,
                persona_similarity: RecordStatus::Recorded,
                conversation_naturalness: RecordStatus::Recorded,
            },
            usable_for_continuation: CheckStatus::Passed,
            artifact_sha256: digest('5'),
        },
        known_limitations: KnownLimitationsEvidence {
            review_status: CheckStatus::Passed,
            document_sha256: digest('6'),
        },
    }
}

fn evaluate(
    evidence: &Rt0ExitEvidence,
    golden: &BoundGoldenReport,
    golden_bytes: &[u8],
    fixture: &support::GoldenFixture,
    release_spec: &[u8],
    candidate: &str,
) -> Result<vpr_evaluation::Rt0ExitReport, Rt0ExitEvidenceError> {
    let live_provider_probe = live_provider_probe(&fixture.provider_state_bytes);
    let live_provider_probe_bytes = serde_json::to_vec(&live_provider_probe).unwrap();
    evaluate_with_probe(
        evidence,
        golden,
        golden_bytes,
        fixture,
        release_spec,
        candidate,
        (&live_provider_probe, &live_provider_probe_bytes),
    )
}

fn evaluate_with_probe(
    evidence: &Rt0ExitEvidence,
    golden: &BoundGoldenReport,
    golden_bytes: &[u8],
    fixture: &support::GoldenFixture,
    release_spec: &[u8],
    candidate: &str,
    live_provider_probe: (&LiveProviderProbeReceipt, &[u8]),
) -> Result<vpr_evaluation::Rt0ExitReport, Rt0ExitEvidenceError> {
    let conversation_attempt_bytes = conversation_attempt_bytes(&fixture.provider_state_bytes);
    let bound_session_aggregate = bound_session_aggregate(&fixture.provider_state_bytes);
    let bound_session_aggregate_bytes = serde_json::to_vec(&bound_session_aggregate).unwrap();
    evaluate_with_runtime(
        evidence,
        (golden, golden_bytes),
        fixture,
        release_spec,
        candidate,
        live_provider_probe,
        (
            &conversation_attempt_bytes,
            &bound_session_aggregate,
            &bound_session_aggregate_bytes,
        ),
    )
}

fn evaluate_with_runtime(
    evidence: &Rt0ExitEvidence,
    golden: (&BoundGoldenReport, &[u8]),
    fixture: &support::GoldenFixture,
    release_spec: &[u8],
    candidate: &str,
    live_provider_probe: (&LiveProviderProbeReceipt, &[u8]),
    runtime: (&[u8], &BoundLabSessionEvidenceAggregate, &[u8]),
) -> Result<vpr_evaluation::Rt0ExitReport, Rt0ExitEvidenceError> {
    let session_snapshot_bytes = session_snapshot_bytes();
    evaluate_with_runtime_snapshot(
        evidence,
        golden,
        fixture,
        release_spec,
        candidate,
        live_provider_probe,
        (runtime.0, runtime.1, runtime.2, &session_snapshot_bytes),
    )
}

fn evaluate_with_runtime_snapshot(
    evidence: &Rt0ExitEvidence,
    golden: (&BoundGoldenReport, &[u8]),
    fixture: &support::GoldenFixture,
    release_spec: &[u8],
    candidate: &str,
    live_provider_probe: (&LiveProviderProbeReceipt, &[u8]),
    runtime: (
        &[u8],
        &BoundLabSessionEvidenceAggregate,
        &[u8],
        &[u8],
    ),
) -> Result<vpr_evaluation::Rt0ExitReport, Rt0ExitEvidenceError> {
    let session_snapshot_artifacts = [runtime.3];
    let exit_bytes = serde_json::to_vec(evidence).unwrap();
    evaluate_rt0_exit_evidence(
        evidence,
        golden.0,
        Rt0ExitVerificationContext {
            exit_evidence_bytes: &exit_bytes,
            golden_report_bytes: golden.1,
            golden_evidence_bundle: &fixture.bundle,
            golden_evidence_bytes: &fixture.bundle_bytes,
            provider_state: &fixture.provider_state,
            provider_state_bytes: &fixture.provider_state_bytes,
            live_provider_probe: live_provider_probe.0,
            live_provider_probe_bytes: live_provider_probe.1,
            conversation_attempt_bytes: runtime.0,
            bound_session_aggregate: runtime.1,
            bound_session_aggregate_bytes: runtime.2,
            session_snapshot_artifacts: &session_snapshot_artifacts,
            release_spec_bytes: release_spec,
            exact_candidate_sha: candidate,
        },
    )
}

#[test]
fn exact_threshold_real_evidence_can_pass_without_inventing_provider_charge() {
    let fixture = golden_fixture();
    let golden = fixture.report.clone();
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    assert!(
        bound_session_aggregate(&fixture.provider_state_bytes)
            .aggregate
            .canonical_playback_proven
    );
    let report = evaluate(
        &evidence,
        &golden,
        &golden_bytes,
        &fixture,
        RELEASE_SPEC,
        CANDIDATE,
    )
    .unwrap();
    assert!(report.ready);
    assert!(report.failures.is_empty());
    assert_eq!(report.measured_cost_per_minute_microunits, Some(6_000));
    assert_eq!(report.golden_report_sha256, sha256_hex(&golden_bytes));
}

#[test]
fn every_quality_threshold_fails_when_exceeded_by_one_millisecond() {
    let fixture = golden_fixture();
    let golden = fixture.report.clone();
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let mut evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    evidence.quality.text_first_meaningful_response.p50 = 1_001;
    evidence.quality.first_meaningful_audio.p95 = 3_001;
    evidence.quality.interruption_stop.p95 = 501;
    evidence.quality.first_useful_video.p95 = 2_501;
    evidence.quality.recoverable_reconnect.p95 = 5_001;
    let report = evaluate(
        &evidence,
        &golden,
        &golden_bytes,
        &fixture,
        RELEASE_SPEC,
        CANDIDATE,
    )
    .unwrap();
    for required in [
        Rt0ExitFailureCode::TextLatencyExceeded,
        Rt0ExitFailureCode::AudioLatencyExceeded,
        Rt0ExitFailureCode::InterruptionLatencyExceeded,
        Rt0ExitFailureCode::VideoLatencyExceeded,
        Rt0ExitFailureCode::ReconnectLatencyExceeded,
    ] {
        assert!(report.failures.contains(&required), "missing {required:?}");
    }
    assert!(!report.ready);
}

#[test]
fn av_sync_threshold_is_evaluated_from_recomputed_session_distribution() {
    let fixture = golden_fixture();
    let golden = fixture.report.clone();
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let snapshot = session_snapshot_bytes_with_av_sync([40, 60, 121]);
    let bound = bind_owner_lab_session_evidence(
        &[snapshot.as_slice()],
        &fixture.provider_state_bytes,
        CANDIDATE,
    )
    .unwrap();
    let bound_bytes = serde_json::to_vec(&bound).unwrap();
    let conversation_attempt = conversation_attempt_bytes(&fixture.provider_state_bytes);
    let probe = live_provider_probe(&fixture.provider_state_bytes);
    let probe_bytes = serde_json::to_vec(&probe).unwrap();
    let mut evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    evidence.bound_session_aggregate_sha256 = sha256_hex(&bound_bytes);
    evidence.quality.av_sync_absolute_offset = LatencyDistributionMillis {
        samples: 3,
        p50: 60,
        p95: 121,
    };
    let report = evaluate_with_runtime_snapshot(
        &evidence,
        (&golden, &golden_bytes),
        &fixture,
        RELEASE_SPEC,
        CANDIDATE,
        (&probe, &probe_bytes),
        (&conversation_attempt, &bound, &bound_bytes, &snapshot),
    )
    .unwrap();
    assert!(
        report
            .failures
            .contains(&Rt0ExitFailureCode::AvSyncExceeded)
    );
    assert!(!report.ready);
}

#[test]
fn av_sync_quality_must_match_recomputed_session_distribution() {
    let fixture = golden_fixture();
    let golden = fixture.report.clone();
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let mut evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    evidence.quality.av_sync_absolute_offset.p50 = 59;
    assert_eq!(
        evaluate(
            &evidence,
            &golden,
            &golden_bytes,
            &fixture,
            RELEASE_SPEC,
            CANDIDATE,
        ),
        Err(Rt0ExitEvidenceError::RuntimeEvidenceInvalid)
    );
}

#[test]
fn mock_or_incomplete_evidence_never_closes_rt0() {
    let fixture = golden_fixture();
    let golden = fixture.report.clone();
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let mut evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    evidence.conversations.owner.origin = EvidenceOrigin::Mock;
    evidence.conversations.visitor.origin = EvidenceOrigin::Synthetic;
    evidence.acceptance.origin = EvidenceOrigin::Mock;
    evidence.quality.origin = EvidenceOrigin::Mock;
    evidence.cost.origin = EvidenceOrigin::Synthetic;
    evidence.cost.measured_cost_microunits = None;
    evidence.privacy_permissions.origin = EvidenceOrigin::Mock;
    evidence
        .privacy_permissions
        .accepted_private_context_leakage = 1;
    evidence
        .privacy_permissions
        .accepted_false_owner_attribution = 1;
    evidence.human_evaluation.origin = EvidenceOrigin::Synthetic;
    evidence.human_evaluation.usable_for_continuation = CheckStatus::Failed;
    evidence.known_limitations.review_status = CheckStatus::Failed;
    let report = evaluate(
        &evidence,
        &golden,
        &golden_bytes,
        &fixture,
        RELEASE_SPEC,
        CANDIDATE,
    )
    .unwrap();
    for required in [
        Rt0ExitFailureCode::OwnerConversationNotReal,
        Rt0ExitFailureCode::VisitorConversationNotReal,
        Rt0ExitFailureCode::AcceptanceEvidenceNotReal,
        Rt0ExitFailureCode::QualityEvidenceNotReal,
        Rt0ExitFailureCode::CostEvidenceNotReal,
        Rt0ExitFailureCode::CostNotMeasured,
        Rt0ExitFailureCode::PrivacyEvidenceNotReal,
        Rt0ExitFailureCode::PrivateContextLeakageAccepted,
        Rt0ExitFailureCode::FalseOwnerAttributionAccepted,
        Rt0ExitFailureCode::HumanEvaluationNotReal,
        Rt0ExitFailureCode::HumanEvaluationNotUsable,
        Rt0ExitFailureCode::KnownLimitationsNotReviewed,
    ] {
        assert!(report.failures.contains(&required), "missing {required:?}");
    }
    assert!(!report.ready);
}

#[test]
fn stale_cross_candidate_or_tampered_artifacts_fail_structurally() {
    let fixture = golden_fixture();
    let golden = fixture.report.clone();
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    assert_eq!(
        evaluate(
            &evidence,
            &golden,
            &golden_bytes,
            &fixture,
            RELEASE_SPEC,
            &"2".repeat(40)
        ),
        Err(Rt0ExitEvidenceError::CandidateShaMismatch)
    );
    assert_eq!(
        evaluate(
            &evidence,
            &golden,
            &golden_bytes,
            &fixture,
            b"changed spec",
            CANDIDATE
        ),
        Err(Rt0ExitEvidenceError::ReleaseSpecDigestMismatch)
    );
    assert_eq!(
        evaluate(
            &evidence,
            &golden,
            b"tampered report",
            &fixture,
            RELEASE_SPEC,
            CANDIDATE
        ),
        Err(Rt0ExitEvidenceError::GoldenReportDigestMismatch)
    );
    let mut evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    evidence.provider_state_sha256 = digest('9');
    assert_eq!(
        evaluate(
            &evidence,
            &golden,
            &golden_bytes,
            &fixture,
            RELEASE_SPEC,
            CANDIDATE,
        ),
        Err(Rt0ExitEvidenceError::ProviderStateDigestMismatch)
    );
}

#[test]
fn live_provider_probe_is_exact_candidate_bound_and_fail_closed() {
    let fixture = golden_fixture();
    let golden = fixture.report.clone();
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let mut evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    let probe = live_provider_probe(&fixture.provider_state_bytes);
    assert_eq!(
        evaluate_with_probe(
            &evidence,
            &golden,
            &golden_bytes,
            &fixture,
            RELEASE_SPEC,
            CANDIDATE,
            (&probe, b"tampered probe bytes"),
        ),
        Err(Rt0ExitEvidenceError::LiveProviderProbeDigestMismatch)
    );

    let mut wrong_candidate = probe.clone();
    wrong_candidate.candidate_sha = "2".repeat(40);
    let wrong_candidate_bytes = serde_json::to_vec(&wrong_candidate).unwrap();
    evidence.live_provider_probe_sha256 = sha256_hex(&wrong_candidate_bytes);
    assert_eq!(
        evaluate_with_probe(
            &evidence,
            &golden,
            &golden_bytes,
            &fixture,
            RELEASE_SPEC,
            CANDIDATE,
            (&wrong_candidate, &wrong_candidate_bytes),
        ),
        Err(Rt0ExitEvidenceError::LiveProviderProbeCandidateMismatch)
    );

    let mut wrong_provider = probe.clone();
    wrong_provider.provider_state_sha256 = digest('9');
    let wrong_provider_bytes = serde_json::to_vec(&wrong_provider).unwrap();
    evidence.live_provider_probe_sha256 = sha256_hex(&wrong_provider_bytes);
    assert_eq!(
        evaluate_with_probe(
            &evidence,
            &golden,
            &golden_bytes,
            &fixture,
            RELEASE_SPEC,
            CANDIDATE,
            (&wrong_provider, &wrong_provider_bytes),
        ),
        Err(Rt0ExitEvidenceError::LiveProviderProbeProviderStateMismatch)
    );

    let mut empty_output = probe.clone();
    empty_output.stt.transcript_chars = 0;
    empty_output.llm.output_chars = 0;
    let empty_output_bytes = serde_json::to_vec(&empty_output).unwrap();
    evidence.live_provider_probe_sha256 = sha256_hex(&empty_output_bytes);
    assert_eq!(
        evaluate_with_probe(
            &evidence,
            &golden,
            &golden_bytes,
            &fixture,
            RELEASE_SPEC,
            CANDIDATE,
            (&empty_output, &empty_output_bytes),
        ),
        Err(Rt0ExitEvidenceError::LiveProviderProbeInvalid)
    );

    let mut forged_conversation = probe;
    forged_conversation.conversation_evidence = true;
    let forged_bytes = serde_json::to_vec(&forged_conversation).unwrap();
    evidence.live_provider_probe_sha256 = sha256_hex(&forged_bytes);
    assert_eq!(
        evaluate_with_probe(
            &evidence,
            &golden,
            &golden_bytes,
            &fixture,
            RELEASE_SPEC,
            CANDIDATE,
            (&forged_conversation, &forged_bytes),
        ),
        Err(Rt0ExitEvidenceError::LiveProviderProbeInvalid)
    );
}

#[test]
fn runtime_evidence_is_exact_candidate_provider_bound_and_fail_closed() {
    let fixture = golden_fixture();
    let golden = fixture.report.clone();
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let probe = live_provider_probe(&fixture.provider_state_bytes);
    let probe_bytes = serde_json::to_vec(&probe).unwrap();
    let base_conversation = conversation_attempt_bytes(&fixture.provider_state_bytes);
    let base_session = bound_session_aggregate(&fixture.provider_state_bytes);
    let base_session_bytes = serde_json::to_vec(&base_session).unwrap();

    let evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    assert_eq!(
        evaluate_with_runtime(
            &evidence,
            (&golden, &golden_bytes),
            &fixture,
            RELEASE_SPEC,
            CANDIDATE,
            (&probe, &probe_bytes),
            (b"tampered", &base_session, &base_session_bytes),
        ),
        Err(Rt0ExitEvidenceError::RuntimeEvidenceDigestMismatch)
    );

    let mut conversation: serde_json::Value = serde_json::from_slice(&base_conversation).unwrap();
    conversation["candidate_sha"] = serde_json::json!("2".repeat(40));
    let conversation_bytes = serde_json::to_vec(&conversation).unwrap();
    let mut evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    evidence.conversation_attempt_sha256 = sha256_hex(&conversation_bytes);
    assert_eq!(
        evaluate_with_runtime(
            &evidence,
            (&golden, &golden_bytes),
            &fixture,
            RELEASE_SPEC,
            CANDIDATE,
            (&probe, &probe_bytes),
            (&conversation_bytes, &base_session, &base_session_bytes),
        ),
        Err(Rt0ExitEvidenceError::RuntimeEvidenceCandidateMismatch)
    );

    let mut session = base_session;
    session.provider_state_sha256 = digest('8');
    let session_bytes = serde_json::to_vec(&session).unwrap();
    let mut evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    evidence.bound_session_aggregate_sha256 = sha256_hex(&session_bytes);
    assert_eq!(
        evaluate_with_runtime(
            &evidence,
            (&golden, &golden_bytes),
            &fixture,
            RELEASE_SPEC,
            CANDIDATE,
            (&probe, &probe_bytes),
            (&base_conversation, &session, &session_bytes),
        ),
        Err(Rt0ExitEvidenceError::RuntimeEvidenceProviderStateMismatch)
    );

    let mut forged = bound_session_aggregate(&fixture.provider_state_bytes);
    forged.aggregate.first_meaningful_audio = Some(distribution(501, 501));
    let forged_bytes = serde_json::to_vec(&forged).unwrap();
    let mut evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    evidence.bound_session_aggregate_sha256 = sha256_hex(&forged_bytes);
    assert_eq!(
        evaluate_with_runtime(
            &evidence,
            (&golden, &golden_bytes),
            &fixture,
            RELEASE_SPEC,
            CANDIDATE,
            (&probe, &probe_bytes),
            (&base_conversation, &forged, &forged_bytes),
        ),
        Err(Rt0ExitEvidenceError::RuntimeEvidenceInvalid)
    );
}

#[test]
fn provider_state_content_is_recomputed_instead_of_trusted_from_golden_report() {
    let fixture = golden_fixture();
    let mut golden = fixture.report.clone();
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    golden.provider_state.providers[0].provider = "tampered-stt".into();
    let exit_bytes = serde_json::to_vec(&evidence).unwrap();
    let provider_state = fixture.provider_state.clone();
    let provider_state_bytes = fixture.provider_state_bytes.clone();
    let live_provider_probe = live_provider_probe(&provider_state_bytes);
    let live_provider_probe_bytes = serde_json::to_vec(&live_provider_probe).unwrap();
    let conversation_attempt_bytes = conversation_attempt_bytes(&provider_state_bytes);
    let bound_session_aggregate = bound_session_aggregate(&provider_state_bytes);
    let bound_session_aggregate_bytes = serde_json::to_vec(&bound_session_aggregate).unwrap();
    let session_snapshot_bytes = session_snapshot_bytes();
    let session_snapshot_artifacts = [session_snapshot_bytes.as_slice()];
    assert_eq!(
        evaluate_rt0_exit_evidence(
            &evidence,
            &golden,
            Rt0ExitVerificationContext {
                exit_evidence_bytes: &exit_bytes,
                golden_report_bytes: &golden_bytes,
                golden_evidence_bundle: &fixture.bundle,
                golden_evidence_bytes: &fixture.bundle_bytes,
                provider_state: &provider_state,
                provider_state_bytes: &provider_state_bytes,
                live_provider_probe: &live_provider_probe,
                live_provider_probe_bytes: &live_provider_probe_bytes,
                conversation_attempt_bytes: &conversation_attempt_bytes,
                bound_session_aggregate: &bound_session_aggregate,
                bound_session_aggregate_bytes: &bound_session_aggregate_bytes,
                session_snapshot_artifacts: &session_snapshot_artifacts,
                release_spec_bytes: RELEASE_SPEC,
                exact_candidate_sha: CANDIDATE,
            },
        ),
        Err(Rt0ExitEvidenceError::ProviderStateMismatch)
    );
}

#[test]
fn malformed_structural_evidence_is_rejected_before_exit_decision() {
    let fixture = golden_fixture();
    let golden = fixture.report.clone();
    let golden_bytes = serde_json::to_vec(&golden).unwrap();

    let mut evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    evidence.conversations.owner.role = ParticipantRole::Visitor;
    assert_eq!(
        evaluate(
            &evidence,
            &golden,
            &golden_bytes,
            &fixture,
            RELEASE_SPEC,
            CANDIDATE,
        ),
        Err(Rt0ExitEvidenceError::ParticipantRoleMismatch)
    );

    let mut evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    evidence.quality.interruption_stop.samples = 0;
    assert_eq!(
        evaluate(
            &evidence,
            &golden,
            &golden_bytes,
            &fixture,
            RELEASE_SPEC,
            CANDIDATE,
        ),
        Err(Rt0ExitEvidenceError::InvalidLatencyDistribution)
    );

    let mut evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    evidence.automated.ci.artifact_sha256 = "not-a-digest".into();
    assert_eq!(
        evaluate(
            &evidence,
            &golden,
            &golden_bytes,
            &fixture,
            RELEASE_SPEC,
            CANDIDATE,
        ),
        Err(Rt0ExitEvidenceError::InvalidArtifactDigest)
    );

    let mut evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    evidence.human_evaluation.rubric_version = "   ".into();
    assert_eq!(
        evaluate(
            &evidence,
            &golden,
            &golden_bytes,
            &fixture,
            RELEASE_SPEC,
            CANDIDATE,
        ),
        Err(Rt0ExitEvidenceError::InvalidHumanRubric)
    );
}

#[test]
fn internally_inconsistent_golden_report_is_not_trusted() {
    let fixture = golden_fixture();
    let mut golden = fixture.report.clone();
    golden.golden.passed = 0;
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    assert_eq!(
        evaluate(
            &evidence,
            &golden,
            &golden_bytes,
            &fixture,
            RELEASE_SPEC,
            CANDIDATE,
        ),
        Err(Rt0ExitEvidenceError::GoldenReportInvalid)
    );

    let fixture = failing_golden_fixture();
    let golden = fixture.report.clone();
    assert_eq!(golden.golden.failed, 1);
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    let report = evaluate(
        &evidence,
        &golden,
        &golden_bytes,
        &fixture,
        RELEASE_SPEC,
        CANDIDATE,
    )
    .unwrap();
    assert_eq!(
        report.failures,
        vec![Rt0ExitFailureCode::GoldenSetNotPassed]
    );
}

#[test]
fn forged_shortened_passing_golden_report_is_rejected_by_recomputation() {
    let fixture = golden_fixture();
    let mut forged = fixture.report.clone();
    forged.golden.total = 1;
    forged.golden.passed = 1;
    forged.golden.failed = 0;
    forged.golden.cases.truncate(1);
    forged.golden.cases[0].passed = true;
    forged.golden.cases[0].failures.clear();
    let forged_bytes = serde_json::to_vec(&forged).unwrap();
    let evidence = passing_evidence(&forged_bytes, &fixture.provider_state_bytes);
    assert_eq!(
        evaluate(
            &evidence,
            &forged,
            &forged_bytes,
            &fixture,
            RELEASE_SPEC,
            CANDIDATE,
        ),
        Err(Rt0ExitEvidenceError::GoldenReportRecomputeMismatch)
    );
}

#[test]
fn golden_report_count_overflow_is_structural_error_not_panic() {
    let fixture = golden_fixture();
    let mut malformed = fixture.report.clone();
    malformed.golden.passed = usize::MAX;
    malformed.golden.failed = 1;
    let malformed_bytes = serde_json::to_vec(&malformed).unwrap();
    let evidence = passing_evidence(&malformed_bytes, &fixture.provider_state_bytes);
    assert_eq!(
        evaluate(
            &evidence,
            &malformed,
            &malformed_bytes,
            &fixture,
            RELEASE_SPEC,
            CANDIDATE,
        ),
        Err(Rt0ExitEvidenceError::GoldenReportInvalid)
    );
}

#[test]
fn synthetic_exit_example_is_parseable_but_explicitly_non_real() {
    let evidence: Rt0ExitEvidence = serde_json::from_str(include_str!(
        "../../../docs/evaluation/rt0_exit_evidence.synthetic.example.json"
    ))
    .unwrap();
    assert_eq!(
        evidence.conversations.owner.origin,
        EvidenceOrigin::Synthetic
    );
    assert_eq!(
        evidence.conversations.visitor.origin,
        EvidenceOrigin::Synthetic
    );
    assert_eq!(evidence.acceptance.origin, EvidenceOrigin::Synthetic);
    assert_eq!(evidence.quality.origin, EvidenceOrigin::Synthetic);
    assert_eq!(evidence.cost.origin, EvidenceOrigin::Synthetic);
    assert_eq!(
        evidence.privacy_permissions.origin,
        EvidenceOrigin::Synthetic
    );
    assert_eq!(evidence.human_evaluation.origin, EvidenceOrigin::Synthetic);
}

#[test]
fn duplicate_golden_case_ids_are_rejected_as_tampered_report() {
    let fixture = golden_fixture();
    let mut golden = fixture.report.clone();
    golden.golden.total = 2;
    golden.golden.passed = 2;
    golden.golden.cases.push(golden.golden.cases[0].clone());
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    assert_eq!(
        evaluate(
            &evidence,
            &golden,
            &golden_bytes,
            &fixture,
            RELEASE_SPEC,
            CANDIDATE,
        ),
        Err(Rt0ExitEvidenceError::GoldenReportInvalid)
    );
}

#[test]
fn incomplete_human_dimensions_fail_exit_without_corrupting_evidence_structure() {
    let fixture = golden_fixture();
    let golden = fixture.report.clone();
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let mut evidence = passing_evidence(&golden_bytes, &fixture.provider_state_bytes);
    evidence.human_evaluation.dimensions.persona_similarity = RecordStatus::Missing;
    let report = evaluate(
        &evidence,
        &golden,
        &golden_bytes,
        &fixture,
        RELEASE_SPEC,
        CANDIDATE,
    )
    .unwrap();
    assert!(!report.ready);
    assert_eq!(
        report.failures,
        vec![Rt0ExitFailureCode::HumanEvaluationIncomplete]
    );
}
