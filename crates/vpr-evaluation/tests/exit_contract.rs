use vpr_evaluation::{
    AcceptanceEvidence, ArtifactCheckEvidence, AutomatedEvidence, BoundGoldenReport, CheckStatus,
    ConversationEvidence, ConversationPairEvidence, CostEvidence, EvidenceBinding, EvidenceOrigin,
    GoldenCaseResult, GoldenFailureCode, GoldenReport, HumanDimensions, HumanEvaluationEvidence,
    KnownLimitationsEvidence, LatencyDistributionMillis, ParticipantRole,
    PrivacyPermissionEvidence, ProviderRole, ProviderStateBinding, ProviderStateManifest,
    QualityEvidence, RT0_EVIDENCE_BINDING_SCHEMA, RT0_EXIT_EVIDENCE_SCHEMA, RT0_GOLDEN_SCHEMA,
    RT0_PROVIDER_STATE_SCHEMA, RecordStatus, Rt0ExitEvidence, Rt0ExitEvidenceError,
    Rt0ExitFailureCode, Rt0ExitVerificationContext, evaluate_rt0_exit_evidence, sha256_hex,
};

const CANDIDATE: &str = "1111111111111111111111111111111111111111";
const RELEASE_SPEC: &[u8] = b"rt0 release spec contract";

fn digest(byte: char) -> String {
    byte.to_string().repeat(64)
}

fn provider(role: ProviderRole, name: &str, fingerprint: char) -> ProviderStateBinding {
    ProviderStateBinding {
        role,
        provider: name.into(),
        model_or_representation: "contract-v1".into(),
        configuration_fingerprint_sha256: digest(fingerprint),
    }
}

fn provider_state() -> ProviderStateManifest {
    ProviderStateManifest {
        schema_version: RT0_PROVIDER_STATE_SCHEMA.into(),
        providers: vec![
            provider(ProviderRole::Stt, "contract-stt", 'a'),
            provider(ProviderRole::Llm, "contract-llm", 'b'),
            provider(ProviderRole::Avatar, "contract-avatar", 'c'),
        ],
    }
}

fn provider_state_digest() -> String {
    sha256_hex(&serde_json::to_vec(&provider_state()).unwrap())
}

fn golden_report() -> BoundGoldenReport {
    BoundGoldenReport {
        evidence_input_sha256: digest('f'),
        binding: EvidenceBinding {
            schema_version: RT0_EVIDENCE_BINDING_SCHEMA.into(),
            candidate_sha: CANDIDATE.into(),
            release_spec_sha256: sha256_hex(RELEASE_SPEC),
            suite_sha256: digest('e'),
            provider_state_sha256: provider_state_digest(),
        },
        provider_state: provider_state(),
        golden: GoldenReport {
            schema_version: RT0_GOLDEN_SCHEMA.into(),
            suite_id: "rt0.contract".into(),
            total: 1,
            passed: 1,
            failed: 0,
            cases: vec![GoldenCaseResult {
                case_id: "contract.case".into(),
                passed: true,
                failures: vec![],
            }],
        },
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

fn passing_evidence(golden_bytes: &[u8]) -> Rt0ExitEvidence {
    Rt0ExitEvidence {
        schema_version: RT0_EXIT_EVIDENCE_SCHEMA.into(),
        candidate_sha: CANDIDATE.into(),
        release_spec_sha256: sha256_hex(RELEASE_SPEC),
        golden_report_sha256: sha256_hex(golden_bytes),
        provider_state_sha256: provider_state_digest(),
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
            av_sync_absolute_offset: distribution(60, 120),
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
    release_spec: &[u8],
    candidate: &str,
) -> Result<vpr_evaluation::Rt0ExitReport, Rt0ExitEvidenceError> {
    let exit_bytes = serde_json::to_vec(evidence).unwrap();
    let provider_state = golden.provider_state.clone();
    let provider_state_bytes = serde_json::to_vec(&provider_state).unwrap();
    evaluate_rt0_exit_evidence(
        evidence,
        golden,
        Rt0ExitVerificationContext {
            exit_evidence_bytes: &exit_bytes,
            golden_report_bytes: golden_bytes,
            provider_state: &provider_state,
            provider_state_bytes: &provider_state_bytes,
            release_spec_bytes: release_spec,
            exact_candidate_sha: candidate,
        },
    )
}

#[test]
fn exact_threshold_real_evidence_can_pass_without_inventing_provider_charge() {
    let golden = golden_report();
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let evidence = passing_evidence(&golden_bytes);
    let report = evaluate(&evidence, &golden, &golden_bytes, RELEASE_SPEC, CANDIDATE).unwrap();
    assert!(report.ready);
    assert!(report.failures.is_empty());
    assert_eq!(report.measured_cost_per_minute_microunits, Some(6_000));
    assert_eq!(report.golden_report_sha256, sha256_hex(&golden_bytes));
}

#[test]
fn every_quality_threshold_fails_when_exceeded_by_one_millisecond() {
    let golden = golden_report();
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let mut evidence = passing_evidence(&golden_bytes);
    evidence.quality.text_first_meaningful_response.p50 = 1_001;
    evidence.quality.first_meaningful_audio.p95 = 3_001;
    evidence.quality.interruption_stop.p95 = 501;
    evidence.quality.first_useful_video.p95 = 2_501;
    evidence.quality.av_sync_absolute_offset.p95 = 121;
    evidence.quality.recoverable_reconnect.p95 = 5_001;
    let report = evaluate(&evidence, &golden, &golden_bytes, RELEASE_SPEC, CANDIDATE).unwrap();
    for required in [
        Rt0ExitFailureCode::TextLatencyExceeded,
        Rt0ExitFailureCode::AudioLatencyExceeded,
        Rt0ExitFailureCode::InterruptionLatencyExceeded,
        Rt0ExitFailureCode::VideoLatencyExceeded,
        Rt0ExitFailureCode::AvSyncExceeded,
        Rt0ExitFailureCode::ReconnectLatencyExceeded,
    ] {
        assert!(report.failures.contains(&required), "missing {required:?}");
    }
    assert!(!report.ready);
}

#[test]
fn mock_or_incomplete_evidence_never_closes_rt0() {
    let golden = golden_report();
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let mut evidence = passing_evidence(&golden_bytes);
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
    let report = evaluate(&evidence, &golden, &golden_bytes, RELEASE_SPEC, CANDIDATE).unwrap();
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
    let golden = golden_report();
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let evidence = passing_evidence(&golden_bytes);
    assert_eq!(
        evaluate(
            &evidence,
            &golden,
            &golden_bytes,
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
            RELEASE_SPEC,
            CANDIDATE
        ),
        Err(Rt0ExitEvidenceError::GoldenReportDigestMismatch)
    );
    let mut evidence = passing_evidence(&golden_bytes);
    evidence.provider_state_sha256 = digest('9');
    assert_eq!(
        evaluate(&evidence, &golden, &golden_bytes, RELEASE_SPEC, CANDIDATE),
        Err(Rt0ExitEvidenceError::ProviderStateDigestMismatch)
    );
}

#[test]
fn provider_state_content_is_recomputed_instead_of_trusted_from_golden_report() {
    let mut golden = golden_report();
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let evidence = passing_evidence(&golden_bytes);
    golden.provider_state.providers[0].provider = "tampered-stt".into();
    let exit_bytes = serde_json::to_vec(&evidence).unwrap();
    let provider_state = provider_state();
    let provider_state_bytes = serde_json::to_vec(&provider_state).unwrap();
    assert_eq!(
        evaluate_rt0_exit_evidence(
            &evidence,
            &golden,
            Rt0ExitVerificationContext {
                exit_evidence_bytes: &exit_bytes,
                golden_report_bytes: &golden_bytes,
                provider_state: &provider_state,
                provider_state_bytes: &provider_state_bytes,
                release_spec_bytes: RELEASE_SPEC,
                exact_candidate_sha: CANDIDATE,
            },
        ),
        Err(Rt0ExitEvidenceError::ProviderStateMismatch)
    );
}

#[test]
fn malformed_structural_evidence_is_rejected_before_exit_decision() {
    let golden = golden_report();
    let golden_bytes = serde_json::to_vec(&golden).unwrap();

    let mut evidence = passing_evidence(&golden_bytes);
    evidence.conversations.owner.role = ParticipantRole::Visitor;
    assert_eq!(
        evaluate(&evidence, &golden, &golden_bytes, RELEASE_SPEC, CANDIDATE),
        Err(Rt0ExitEvidenceError::ParticipantRoleMismatch)
    );

    let mut evidence = passing_evidence(&golden_bytes);
    evidence.quality.interruption_stop.samples = 0;
    assert_eq!(
        evaluate(&evidence, &golden, &golden_bytes, RELEASE_SPEC, CANDIDATE),
        Err(Rt0ExitEvidenceError::InvalidLatencyDistribution)
    );

    let mut evidence = passing_evidence(&golden_bytes);
    evidence.automated.ci.artifact_sha256 = "not-a-digest".into();
    assert_eq!(
        evaluate(&evidence, &golden, &golden_bytes, RELEASE_SPEC, CANDIDATE),
        Err(Rt0ExitEvidenceError::InvalidArtifactDigest)
    );

    let mut evidence = passing_evidence(&golden_bytes);
    evidence.human_evaluation.rubric_version = "   ".into();
    assert_eq!(
        evaluate(&evidence, &golden, &golden_bytes, RELEASE_SPEC, CANDIDATE),
        Err(Rt0ExitEvidenceError::InvalidHumanRubric)
    );
}

#[test]
fn internally_inconsistent_golden_report_is_not_trusted() {
    let mut golden = golden_report();
    golden.golden.passed = 0;
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let evidence = passing_evidence(&golden_bytes);
    assert_eq!(
        evaluate(&evidence, &golden, &golden_bytes, RELEASE_SPEC, CANDIDATE),
        Err(Rt0ExitEvidenceError::GoldenReportInvalid)
    );

    let mut golden = golden_report();
    golden.golden.passed = 0;
    golden.golden.failed = 1;
    golden.golden.cases[0].passed = false;
    golden.golden.cases[0].failures = vec![GoldenFailureCode::MissingRequiredText];
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let evidence = passing_evidence(&golden_bytes);
    let report = evaluate(&evidence, &golden, &golden_bytes, RELEASE_SPEC, CANDIDATE).unwrap();
    assert_eq!(
        report.failures,
        vec![Rt0ExitFailureCode::GoldenSetNotPassed]
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
    let mut golden = golden_report();
    golden.golden.total = 2;
    golden.golden.passed = 2;
    golden.golden.cases.push(golden.golden.cases[0].clone());
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let evidence = passing_evidence(&golden_bytes);
    assert_eq!(
        evaluate(&evidence, &golden, &golden_bytes, RELEASE_SPEC, CANDIDATE),
        Err(Rt0ExitEvidenceError::GoldenReportInvalid)
    );
}

#[test]
fn incomplete_human_dimensions_fail_exit_without_corrupting_evidence_structure() {
    let golden = golden_report();
    let golden_bytes = serde_json::to_vec(&golden).unwrap();
    let mut evidence = passing_evidence(&golden_bytes);
    evidence.human_evaluation.dimensions.persona_similarity = RecordStatus::Missing;
    let report = evaluate(&evidence, &golden, &golden_bytes, RELEASE_SPEC, CANDIDATE).unwrap();
    assert!(!report.ready);
    assert_eq!(
        report.failures,
        vec![Rt0ExitFailureCode::HumanEvaluationIncomplete]
    );
}
