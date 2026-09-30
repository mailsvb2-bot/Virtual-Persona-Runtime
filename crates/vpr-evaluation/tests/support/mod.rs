use vpr_evaluation::{
    BoundGoldenReport, EvidenceBinding, EvidenceVerificationContext, GoldenEvidenceBundle,
    GoldenSuite, ProviderRole, ProviderStateBinding, ProviderStateManifest,
    RT0_EVIDENCE_BINDING_SCHEMA, RT0_PROVIDER_STATE_SCHEMA, evaluate_bound_golden_suite,
    sha256_hex,
};

pub const SUITE_BYTES: &[u8] =
    include_bytes!("../../../../docs/evaluation/rt0_golden_minimum.json");

#[allow(dead_code)]
pub struct GoldenFixture {
    pub provider_state: ProviderStateManifest,
    pub provider_state_bytes: Vec<u8>,
    pub bundle: GoldenEvidenceBundle,
    pub bundle_bytes: Vec<u8>,
    pub report: BoundGoldenReport,
}

pub fn fixture(release_spec: &[u8], candidate: &str) -> GoldenFixture {
    let provider_state = provider_state();
    let provider_state_bytes = serde_json::to_vec_pretty(&provider_state).unwrap();
    let binding = EvidenceBinding {
        schema_version: RT0_EVIDENCE_BINDING_SCHEMA.into(),
        candidate_sha: candidate.into(),
        release_spec_sha256: sha256_hex(release_spec),
        suite_sha256: sha256_hex(SUITE_BYTES),
        provider_state_sha256: sha256_hex(&provider_state_bytes),
    };
    let observations: serde_json::Value =
        serde_json::from_slice(include_bytes!("passing_observations.json")).unwrap();
    let bundle_value = serde_json::json!({
        "binding": binding,
        "observations": observations,
    });
    let bundle_bytes = serde_json::to_vec_pretty(&bundle_value).unwrap();
    let bundle: GoldenEvidenceBundle = serde_json::from_slice(&bundle_bytes).unwrap();
    let suite: GoldenSuite = serde_json::from_slice(SUITE_BYTES).unwrap();
    let report = evaluate_bound_golden_suite(
        &suite,
        &bundle,
        EvidenceVerificationContext {
            suite_bytes: SUITE_BYTES,
            release_spec_bytes: release_spec,
            provider_state: &provider_state,
            provider_state_bytes: &provider_state_bytes,
            evidence_bytes: &bundle_bytes,
            exact_candidate_sha: candidate,
        },
    )
    .unwrap();
    GoldenFixture {
        provider_state,
        provider_state_bytes,
        bundle,
        bundle_bytes,
        report,
    }
}

#[allow(dead_code)]
pub struct OwnerGoldenFixture {
    pub suite: GoldenSuite,
    pub suite_bytes: Vec<u8>,
    pub bundle: GoldenEvidenceBundle,
    pub bundle_bytes: Vec<u8>,
    pub report: BoundGoldenReport,
    pub report_bytes: Vec<u8>,
}

#[allow(dead_code)]
pub fn owner_fixture(
    release_spec: &[u8],
    candidate: &str,
    provider_state: &ProviderStateManifest,
    provider_state_bytes: &[u8],
) -> OwnerGoldenFixture {
    let suite_value = serde_json::json!({
        "schema_version": "rt0-golden-0.1",
        "suite_id": "rt0.owner.contract-private-v1",
        "cases": [
            {
                "id": "owner.private.fact",
                "actor": "owner",
                "prompt_ru": "Закрытый вопрос о подтверждённом факте владельца.",
                "expectations": [{"kind":"response_contains_all","tokens":["PRIVATE_OWNER_FACT"]}]
            },
            {
                "id": "owner.private.attribution",
                "actor": "owner",
                "prompt_ru": "Назови подтверждённое владельцем мнение.",
                "expectations": [{"kind":"owner_attribution","eligible":true}]
            },
            {
                "id": "owner.private.identity",
                "actor": "owner",
                "prompt_ru": "Проверь стабильность идентичности Persona.",
                "expectations": [{"kind":"persona_identity_stable"}]
            }
        ]
    });
    let suite_bytes = serde_json::to_vec_pretty(&suite_value).unwrap();
    let suite: GoldenSuite = serde_json::from_slice(&suite_bytes).unwrap();
    let binding = EvidenceBinding {
        schema_version: RT0_EVIDENCE_BINDING_SCHEMA.into(),
        candidate_sha: candidate.into(),
        release_spec_sha256: sha256_hex(release_spec),
        suite_sha256: sha256_hex(&suite_bytes),
        provider_state_sha256: sha256_hex(provider_state_bytes),
    };
    let bundle_value = serde_json::json!({
        "binding": binding,
        "observations": [
            {
                "case_id":"owner.private.fact",
                "response_text":"PRIVATE_OWNER_FACT",
                "owner_attribution":null,
                "reason_code":null,
                "unplayed_tail_spoken":null,
                "persona_id_before":null,
                "persona_id_after":null
            },
            {
                "case_id":"owner.private.attribution",
                "response_text":null,
                "owner_attribution":{
                    "source":"owner",
                    "verification":"owner_verified",
                    "derivation":"direct"
                },
                "reason_code":null,
                "unplayed_tail_spoken":null,
                "persona_id_before":null,
                "persona_id_after":null
            },
            {
                "case_id":"owner.private.identity",
                "response_text":null,
                "owner_attribution":null,
                "reason_code":null,
                "unplayed_tail_spoken":null,
                "persona_id_before":"persona-owner-contract",
                "persona_id_after":"persona-owner-contract"
            }
        ]
    });
    let bundle_bytes = serde_json::to_vec_pretty(&bundle_value).unwrap();
    let bundle: GoldenEvidenceBundle = serde_json::from_slice(&bundle_bytes).unwrap();
    let report = evaluate_bound_golden_suite(
        &suite,
        &bundle,
        EvidenceVerificationContext {
            suite_bytes: &suite_bytes,
            release_spec_bytes: release_spec,
            provider_state,
            provider_state_bytes,
            evidence_bytes: &bundle_bytes,
            exact_candidate_sha: candidate,
        },
    )
    .unwrap();
    let report_bytes = serde_json::to_vec_pretty(&report).unwrap();
    OwnerGoldenFixture {
        suite,
        suite_bytes,
        bundle,
        bundle_bytes,
        report,
        report_bytes,
    }
}

fn provider_state() -> ProviderStateManifest {
    ProviderStateManifest {
        schema_version: RT0_PROVIDER_STATE_SCHEMA.into(),
        providers: vec![
            provider(ProviderRole::Stt, "contract-stt", b"stt-config"),
            provider(ProviderRole::Llm, "contract-llm", b"llm-config"),
            provider(ProviderRole::Avatar, "contract-avatar", b"avatar-config"),
        ],
    }
}

fn provider(role: ProviderRole, name: &str, config: &[u8]) -> ProviderStateBinding {
    ProviderStateBinding {
        role,
        provider: name.into(),
        model_or_representation: "contract-v1".into(),
        configuration_fingerprint_sha256: sha256_hex(config),
    }
}
