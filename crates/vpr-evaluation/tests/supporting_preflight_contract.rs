use serde_json::{Value, json};
use vpr_evaluation::{
    CheckStatus, Rt0SupportingPreflightArtifacts, Rt0SupportingPreflightError,
    preflight_rt0_supporting_artifacts, sha256_hex,
};

fn candidate() -> String {
    "1".repeat(40)
}

fn provider_state() -> Vec<u8> {
    include_bytes!("../../../docs/evaluation/rt0_provider_state.synthetic.example.json").to_vec()
}

#[derive(Clone)]
struct Fixture {
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

impl Fixture {
    fn valid() -> Self {
        let candidate = candidate();
        let provider_state = provider_state();
        let provider = sha256_hex(&provider_state);
        let distribution = json!({"samples":3,"p50":100,"p95":200});
        Self {
            ci: bytes(&json!({"status":"passed","candidate_sha":candidate})),
            e2e: bytes(&json!({"status":"passed","candidate_sha":candidate})),
            owner: bytes(&json!({
                "origin":"real",
                "role":"owner",
                "russian":"passed",
                "voice":"passed",
                "video":"passed",
                "completed_turns":1,
                "interruption_exercised":"passed",
                "candidate_sha":candidate,
                "provider_state_sha256":provider
            })),
            visitor: bytes(&json!({
                "origin":"real",
                "role":"visitor",
                "russian":"passed",
                "voice":"passed",
                "video":"passed",
                "completed_turns":1,
                "interruption_exercised":"failed",
                "candidate_sha":candidate,
                "provider_state_sha256":provider
            })),
            acceptance: bytes(&json!({
                "origin":"real",
                "owner_happy_path":"passed",
                "visitor_happy_path":"passed",
                "correction_path":"passed",
                "failure_recovery_path":"failed",
                "revoke_deny_path":"passed",
                "candidate_sha":candidate,
                "provider_state_sha256":provider
            })),
            quality: bytes(&json!({
                "origin":"real",
                "text_first_meaningful_response":distribution,
                "first_meaningful_audio":distribution,
                "interruption_stop":distribution,
                "first_useful_video":distribution,
                "av_sync_absolute_offset":distribution,
                "recoverable_reconnect":distribution,
                "candidate_sha":candidate,
                "provider_state_sha256":provider
            })),
            cost: bytes(&json!({
                "origin":"real",
                "measured_duration_millis":60000,
                "measured_cost_microunits":null,
                "provider_charge_microunits":null,
                "candidate_sha":candidate,
                "provider_state_sha256":provider
            })),
            privacy: bytes(&json!({
                "origin":"real",
                "permission_suite":"failed",
                "accepted_private_context_leakage":1,
                "accepted_false_owner_attribution":0,
                "revocation":"passed",
                "egress_denial":"passed",
                "candidate_sha":candidate,
                "provider_state_sha256":provider
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
                "candidate_sha":candidate,
                "provider_state_sha256":provider
            })),
            limitations:
                b"RT0-Review-Status: failed\n\nReviewed limitations for this candidate.\n"
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

fn bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec_pretty(&value).unwrap()
}

fn mutate(bytes: &mut Vec<u8>, path: &[&str], value: Value) {
    let mut parsed: Value = serde_json::from_slice(bytes).unwrap();
    let mut current = &mut parsed;
    for key in &path[..path.len() - 1] {
        current = current.get_mut(*key).unwrap();
    }
    current[path[path.len() - 1]] = value;
    *bytes = serde_json::to_vec_pretty(&parsed).unwrap();
}

#[test]
fn exact_bound_real_supporting_bundle_is_preflight_complete_without_release_ready_claim() {
    let fixture = Fixture::valid();
    let provider_state = provider_state();
    let report =
        preflight_rt0_supporting_artifacts(fixture.as_preflight(), &provider_state, &candidate())
            .unwrap();

    assert!(report.preflight_complete);
    assert_eq!(
        report.known_limitations_review_status,
        CheckStatus::Failed
    );
    assert_eq!(report.provider_state_sha256, sha256_hex(&provider_state));
    assert_eq!(
        report.artifact_digests.human_evaluation,
        sha256_hex(&fixture.human)
    );
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains("\"ready\""));
    assert!(!json.contains("release_ready"));
}

#[test]
fn failed_substantive_results_remain_valid_evidence_for_preflight() {
    let fixture = Fixture::valid();
    assert!(
        preflight_rt0_supporting_artifacts(fixture.as_preflight(), &provider_state(), &candidate())
            .is_ok()
    );
}

#[test]
fn stale_candidate_binding_is_rejected() {
    let mut fixture = Fixture::valid();
    mutate(
        &mut fixture.human,
        &["candidate_sha"],
        json!("2".repeat(40)),
    );
    assert_eq!(
        preflight_rt0_supporting_artifacts(fixture.as_preflight(), &provider_state(), &candidate()),
        Err(Rt0SupportingPreflightError::CandidateMismatch)
    );
}

#[test]
fn stale_provider_state_binding_is_rejected() {
    let mut fixture = Fixture::valid();
    mutate(
        &mut fixture.privacy,
        &["provider_state_sha256"],
        json!("2".repeat(64)),
    );
    assert_eq!(
        preflight_rt0_supporting_artifacts(fixture.as_preflight(), &provider_state(), &candidate()),
        Err(Rt0SupportingPreflightError::ProviderStateMismatch)
    );
}

#[test]
fn synthetic_real_supporting_slot_is_rejected() {
    let mut fixture = Fixture::valid();
    mutate(&mut fixture.acceptance, &["origin"], json!("synthetic"));
    assert_eq!(
        preflight_rt0_supporting_artifacts(fixture.as_preflight(), &provider_state(), &candidate()),
        Err(Rt0SupportingPreflightError::OriginNotReal)
    );
}

#[test]
fn conversation_role_mismatch_is_rejected() {
    let mut fixture = Fixture::valid();
    mutate(&mut fixture.owner, &["role"], json!("visitor"));
    assert_eq!(
        preflight_rt0_supporting_artifacts(fixture.as_preflight(), &provider_state(), &candidate()),
        Err(Rt0SupportingPreflightError::ParticipantRoleMismatch)
    );
}

#[test]
fn malformed_quality_distribution_is_rejected() {
    let mut fixture = Fixture::valid();
    mutate(
        &mut fixture.quality,
        &["first_meaningful_audio", "p50"],
        json!(300),
    );
    mutate(
        &mut fixture.quality,
        &["first_meaningful_audio", "p95"],
        json!(200),
    );
    assert_eq!(
        preflight_rt0_supporting_artifacts(fixture.as_preflight(), &provider_state(), &candidate()),
        Err(Rt0SupportingPreflightError::InvalidQuality)
    );
}

#[test]
fn incomplete_human_review_is_rejected_without_inventing_a_review() {
    let mut fixture = Fixture::valid();
    mutate(&mut fixture.human, &["reviewer_count"], json!(0));
    assert_eq!(
        preflight_rt0_supporting_artifacts(fixture.as_preflight(), &provider_state(), &candidate()),
        Err(Rt0SupportingPreflightError::IncompleteHumanReview)
    );

    let mut fixture = Fixture::valid();
    mutate(
        &mut fixture.human,
        &["dimensions", "voice_similarity"],
        json!("missing"),
    );
    assert_eq!(
        preflight_rt0_supporting_artifacts(fixture.as_preflight(), &provider_state(), &candidate()),
        Err(Rt0SupportingPreflightError::IncompleteHumanReview)
    );
}

#[test]
fn malformed_known_limitations_review_evidence_is_rejected() {
    for bytes in [
        b" \n\t".as_slice(),
        b"# Known limitations\n\nReviewed limitations.\n".as_slice(),
        b"RT0-Review-Status: passed\n".as_slice(),
        b"RT0-Review-Status: unknown\n\nReviewed limitations.\n".as_slice(),
    ] {
        let mut fixture = Fixture::valid();
        fixture.limitations = bytes.to_vec();
        assert_eq!(
            preflight_rt0_supporting_artifacts(
                fixture.as_preflight(),
                &provider_state(),
                &candidate()
            ),
            Err(Rt0SupportingPreflightError::InvalidKnownLimitations)
        );
    }
}
