use vpr_evaluation::{
    EvidenceBinding, EvidenceBindingError, EvidenceVerificationContext, GoldenEvidenceBundle,
    GoldenSuite, ProviderRole, ProviderStateBinding, ProviderStateManifest,
    RT0_EVIDENCE_BINDING_SCHEMA, RT0_PROVIDER_STATE_SCHEMA, evaluate_bound_golden_suite,
    sha256_hex,
};

const CANDIDATE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SUITE_BYTES: &[u8] = include_bytes!("../../../docs/evaluation/rt0_golden_minimum.json");
const RELEASE_SPEC_BYTES: &[u8] = include_bytes!("../../../docs/releases/RT0_RELEASE_SPEC.md");

fn provider(role: ProviderRole, name: &str) -> ProviderStateBinding {
    ProviderStateBinding {
        role,
        provider: name.into(),
        model_or_representation: "contract-v1".into(),
        configuration_fingerprint_sha256: sha256_hex(format!("{name}-config").as_bytes()),
    }
}

fn provider_state() -> ProviderStateManifest {
    ProviderStateManifest {
        schema_version: RT0_PROVIDER_STATE_SCHEMA.into(),
        providers: vec![
            provider(ProviderRole::Stt, "contract-stt"),
            provider(ProviderRole::Llm, "contract-llm"),
            provider(ProviderRole::Avatar, "contract-avatar"),
        ],
    }
}

fn provider_state_bytes(state: &ProviderStateManifest) -> Vec<u8> {
    serde_json::to_vec_pretty(state).unwrap()
}

fn bundle(provider_bytes: &[u8]) -> GoldenEvidenceBundle {
    GoldenEvidenceBundle {
        binding: EvidenceBinding {
            schema_version: RT0_EVIDENCE_BINDING_SCHEMA.into(),
            candidate_sha: CANDIDATE.into(),
            release_spec_sha256: sha256_hex(RELEASE_SPEC_BYTES),
            suite_sha256: sha256_hex(SUITE_BYTES),
            provider_state_sha256: sha256_hex(provider_bytes),
        },
        observations: Vec::new(),
    }
}

fn suite() -> GoldenSuite {
    serde_json::from_slice(SUITE_BYTES).unwrap()
}

fn evaluate(
    state: &ProviderStateManifest,
    provider_bytes: &[u8],
    evidence: &GoldenEvidenceBundle,
    candidate: &str,
) -> Result<vpr_evaluation::BoundGoldenReport, EvidenceBindingError> {
    evaluate_bound_golden_suite(
        &suite(),
        evidence,
        EvidenceVerificationContext {
            suite_bytes: SUITE_BYTES,
            release_spec_bytes: RELEASE_SPEC_BYTES,
            provider_state: state,
            provider_state_bytes: provider_bytes,
            evidence_bytes: b"private evidence artifact",
            exact_candidate_sha: candidate,
        },
    )
}

#[test]
fn exact_candidate_and_provider_state_are_preserved_in_redacted_report() {
    let state = provider_state();
    let provider_bytes = provider_state_bytes(&state);
    let report = evaluate(&state, &provider_bytes, &bundle(&provider_bytes), CANDIDATE).unwrap();
    assert_eq!(report.binding.candidate_sha, CANDIDATE);
    assert_eq!(
        report.evidence_input_sha256,
        sha256_hex(b"private evidence artifact")
    );
    assert_eq!(report.provider_state.providers.len(), 3);
    assert_eq!(report.golden.total, 13);
    assert_eq!(report.golden.failed, 13);
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains("prompt_ru"));
    assert!(!json.contains("response_text"));
}

#[test]
fn stale_candidate_and_artifact_digests_fail_closed() {
    let state = provider_state();
    let provider_bytes = provider_state_bytes(&state);
    let evidence = bundle(&provider_bytes);
    assert_eq!(
        evaluate(
            &state,
            &provider_bytes,
            &evidence,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        ),
        Err(EvidenceBindingError::CandidateShaMismatch)
    );
    assert_eq!(
        evaluate_bound_golden_suite(
            &suite(),
            &evidence,
            EvidenceVerificationContext {
                suite_bytes: b"different suite bytes",
                release_spec_bytes: RELEASE_SPEC_BYTES,
                provider_state: &state,
                provider_state_bytes: &provider_bytes,
                evidence_bytes: b"private evidence artifact",
                exact_candidate_sha: CANDIDATE,
            },
        ),
        Err(EvidenceBindingError::SuiteDigestMismatch)
    );
    assert_eq!(
        evaluate_bound_golden_suite(
            &suite(),
            &evidence,
            EvidenceVerificationContext {
                suite_bytes: SUITE_BYTES,
                release_spec_bytes: b"different release spec",
                provider_state: &state,
                provider_state_bytes: &provider_bytes,
                evidence_bytes: b"private evidence artifact",
                exact_candidate_sha: CANDIDATE,
            },
        ),
        Err(EvidenceBindingError::ReleaseSpecDigestMismatch)
    );
    assert_eq!(
        evaluate_bound_golden_suite(
            &suite(),
            &evidence,
            EvidenceVerificationContext {
                suite_bytes: SUITE_BYTES,
                release_spec_bytes: RELEASE_SPEC_BYTES,
                provider_state: &state,
                provider_state_bytes: b"different provider state bytes",
                evidence_bytes: b"private evidence artifact",
                exact_candidate_sha: CANDIDATE,
            },
        ),
        Err(EvidenceBindingError::ProviderStateDigestMismatch)
    );
}

#[test]
fn provider_state_requires_unique_stt_llm_and_avatar_fingerprints() {
    let mut state = provider_state();
    state.providers.pop();
    let provider_bytes = provider_state_bytes(&state);
    let evidence = bundle(&provider_bytes);
    assert_eq!(
        evaluate(&state, &provider_bytes, &evidence, CANDIDATE),
        Err(EvidenceBindingError::MissingProviderRole)
    );

    let mut state = provider_state();
    state.providers[0].configuration_fingerprint_sha256 = "not-a-digest".into();
    let provider_bytes = provider_state_bytes(&state);
    let evidence = bundle(&provider_bytes);
    assert_eq!(
        evaluate(&state, &provider_bytes, &evidence, CANDIDATE),
        Err(EvidenceBindingError::InvalidProviderFingerprint)
    );
}

#[test]
fn unknown_binding_schema_and_provider_schema_fail_closed() {
    let state = provider_state();
    let provider_bytes = provider_state_bytes(&state);
    let mut evidence = bundle(&provider_bytes);
    evidence.binding.schema_version = "future-binding".into();
    assert_eq!(
        evaluate(&state, &provider_bytes, &evidence, CANDIDATE),
        Err(EvidenceBindingError::UnsupportedBindingSchema)
    );

    let mut state = provider_state();
    state.schema_version = "future-provider-state".into();
    let provider_bytes = provider_state_bytes(&state);
    let evidence = bundle(&provider_bytes);
    assert_eq!(
        evaluate(&state, &provider_bytes, &evidence, CANDIDATE),
        Err(EvidenceBindingError::UnsupportedProviderStateSchema)
    );
}

#[test]
fn provider_state_rejects_unknown_fields_instead_of_hiding_them() {
    let value = serde_json::json!({
        "schema_version": RT0_PROVIDER_STATE_SCHEMA,
        "providers": [{
            "role": "stt",
            "provider": "contract-stt",
            "model_or_representation": "contract-v1",
            "configuration_fingerprint_sha256": sha256_hex(b"config"),
            "api_key": "must-never-be-accepted"
        }]
    });
    assert!(serde_json::from_value::<ProviderStateManifest>(value).is_err());
}
