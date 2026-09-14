use vpr_evaluation::{
    BoundGoldenReport, EvidenceBinding, EvidenceVerificationContext, GoldenEvidenceBundle,
    GoldenSuite, ProviderRole, ProviderStateBinding, ProviderStateManifest,
    RT0_EVIDENCE_BINDING_SCHEMA, RT0_PROVIDER_STATE_SCHEMA, evaluate_bound_golden_suite,
    sha256_hex,
};

pub const SUITE_BYTES: &[u8] =
    include_bytes!("../../../../docs/evaluation/rt0_golden_minimum.json");

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
