use std::collections::{BTreeMap, HashSet};
use std::{env, fs, path::Path, process};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vpr_evaluation::{
    CheckStatus, HumanDimensions, ProviderRole, ProviderStateManifest, RT0_MANUAL_SUPPORTING_FILES,
    RecordStatus, rt0_manual_supporting_scaffold, sha256_hex, validate_candidate_sha,
    validate_provider_state_manifest,
};

#[path = "rt0_supporting_capture/transaction.rs"]
mod transaction;
use transaction::{
    CaptureLock, LockOutcome, TransactionJournal, artifact_digests, write_artifacts_transactional,
};

const INPUT_SCHEMA: &str = "rt0-manual-supporting-observations-0.1";
const RECEIPT_SCHEMA: &str = "rt0-manual-supporting-capture-receipt-0.1";
const REQUIRED_ATTESTATION: &str = "reviewed_real_observations";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManualSupportingInput {
    schema_version: String,
    attestation: String,
    acceptance: AcceptanceInput,
    cost: CostInput,
    privacy_permissions: PrivacyInput,
    human_evaluation: HumanInput,
    known_limitations: KnownLimitationsInput,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptanceInput {
    #[serde(rename = "owner_happy_path")]
    owner_happy: CheckStatus,
    #[serde(rename = "visitor_happy_path")]
    visitor_happy: CheckStatus,
    #[serde(rename = "correction_path")]
    correction: CheckStatus,
    #[serde(rename = "failure_recovery_path")]
    failure_recovery: CheckStatus,
    #[serde(rename = "revoke_deny_path")]
    revoke_deny: CheckStatus,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CostInput {
    estimated_cost_covered_provider_roles: Vec<ProviderRole>,
    provider_charge_covered_provider_roles: Vec<ProviderRole>,
    measured_duration_millis: u64,
    estimated_cost_microunits: Option<u64>,
    provider_charge_microunits: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivacyInput {
    permission_suite: CheckStatus,
    accepted_private_context_leakage: u32,
    accepted_false_owner_attribution: u32,
    revocation: CheckStatus,
    egress_denial: CheckStatus,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HumanInput {
    rubric_version: String,
    reviewer_count: u32,
    dimensions: HumanDimensions,
    usable_for_continuation: CheckStatus,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KnownLimitationsInput {
    review_status: CheckStatus,
    body: String,
}

#[derive(Debug, Serialize)]
struct CaptureReceipt {
    schema_version: &'static str,
    candidate_sha: String,
    provider_state_sha256: String,
    artifact_sha256: BTreeMap<String, String>,
}

fn main() {
    if let Err(code) = run() {
        process::exit(code);
    }
}

fn run() -> Result<(), i32> {
    let args: Vec<String> = env::args().skip(1).collect();
    let [
        input_path,
        supporting_dir,
        provider_state_path,
        candidate_sha,
    ] = args.as_slice()
    else {
        eprintln!(
            "usage: vpr-rt0-supporting-capture <reviewed-observations.json> <supporting-dir> <provider-state.json> <exact-candidate-sha>"
        );
        return Err(2);
    };

    validate_candidate_sha(candidate_sha).map_err(|_| fail("INVALID_CANDIDATE"))?;
    let provider_bytes = read(Path::new(provider_state_path))?;
    let provider: ProviderStateManifest =
        serde_json::from_slice(&provider_bytes).map_err(|_| fail("INVALID_PROVIDER_STATE"))?;
    validate_provider_state_manifest(&provider).map_err(|_| fail("INVALID_PROVIDER_STATE"))?;
    let provider_state_sha256 = sha256_hex(&provider_bytes);

    let input_bytes = read(Path::new(input_path))?;
    let input: ManualSupportingInput =
        serde_json::from_slice(&input_bytes).map_err(|_| fail("INVALID_INPUT"))?;
    validate_input(&input)?;

    let supporting_dir = Path::new(supporting_dir);
    if !supporting_dir.is_dir() {
        return Err(fail("SUPPORTING_DIR_INVALID"));
    }

    let expected = rt0_manual_supporting_scaffold(candidate_sha, &provider_state_sha256);
    let artifacts = build_artifacts(&input, candidate_sha, &provider_state_sha256);
    let journal = TransactionJournal::new(
        candidate_sha,
        &provider_state_sha256,
        &input_bytes,
        &artifacts,
    );
    let lock = match CaptureLock::acquire(supporting_dir, &journal, &expected, &artifacts)? {
        LockOutcome::Acquired(lock) => {
            validate_exact_placeholders(supporting_dir, &expected)?;
            write_artifacts_transactional(supporting_dir, &expected, &artifacts, &journal)?;
            Some(lock)
        }
        LockOutcome::AlreadyCommitted => None,
    };

    let artifact_sha256 = artifact_digests(&artifacts);
    println!(
        "{}",
        serde_json::to_string_pretty(&CaptureReceipt {
            schema_version: RECEIPT_SCHEMA,
            candidate_sha: candidate_sha.clone(),
            provider_state_sha256,
            artifact_sha256,
        })
        .map_err(|_| fail("INTERNAL_ERROR"))?
    );
    drop(lock);
    Ok(())
}

fn validate_input(input: &ManualSupportingInput) -> Result<(), i32> {
    if input.schema_version != INPUT_SCHEMA || input.attestation != REQUIRED_ATTESTATION {
        return Err(fail("INVALID_ATTESTATION"));
    }
    if input.human_evaluation.rubric_version.trim().is_empty()
        || input.human_evaluation.reviewer_count == 0
        || !human_dimensions_complete(&input.human_evaluation.dimensions)
    {
        return Err(fail("HUMAN_REVIEW_INCOMPLETE"));
    }
    let body = input.known_limitations.body.trim();
    if body.is_empty()
        || body
            .lines()
            .any(|line| line.trim_start().starts_with("RT0-Review-Status:"))
    {
        return Err(fail("KNOWN_LIMITATIONS_INVALID"));
    }
    if has_duplicate_roles(&input.cost.estimated_cost_covered_provider_roles)
        || has_duplicate_roles(&input.cost.provider_charge_covered_provider_roles)
    {
        return Err(fail("COST_PROVIDER_ROLE_DUPLICATE"));
    }
    Ok(())
}

fn human_dimensions_complete(dimensions: &HumanDimensions) -> bool {
    [
        dimensions.voice_similarity,
        dimensions.voice_naturalness,
        dimensions.appearance_plausibility,
        dimensions.persona_similarity,
        dimensions.conversation_naturalness,
    ]
    .into_iter()
    .all(|status| status == RecordStatus::Recorded)
}

fn has_duplicate_roles(roles: &[ProviderRole]) -> bool {
    let mut seen = HashSet::new();
    roles.iter().any(|role| !seen.insert(*role))
}

fn validate_exact_placeholders(
    root: &Path,
    expected: &[(&'static str, String)],
) -> Result<(), i32> {
    if expected.len() != RT0_MANUAL_SUPPORTING_FILES.len() {
        return Err(fail("INTERNAL_ERROR"));
    }
    for (name, content) in expected {
        let path = root.join(name);
        let metadata = fs::symlink_metadata(&path).map_err(|_| fail("INPUT_INVALID"))?;
        if !metadata.file_type().is_file() {
            return Err(fail("REVIEWED_ARTIFACT_EXISTS"));
        }
        let existing = read(&path)?;
        if existing != content.as_bytes() {
            return Err(fail("REVIEWED_ARTIFACT_EXISTS"));
        }
    }
    Ok(())
}

fn build_artifacts(
    input: &ManualSupportingInput,
    candidate_sha: &str,
    provider_state_sha256: &str,
) -> Vec<(&'static str, Vec<u8>)> {
    vec![
        (
            "acceptance.json",
            pretty_bytes(&json!({
                "origin": "real",
                "owner_happy_path": input.acceptance.owner_happy,
                "visitor_happy_path": input.acceptance.visitor_happy,
                "correction_path": input.acceptance.correction,
                "failure_recovery_path": input.acceptance.failure_recovery,
                "revoke_deny_path": input.acceptance.revoke_deny,
                "candidate_sha": candidate_sha,
                "provider_state_sha256": provider_state_sha256,
            })),
        ),
        (
            "cost.json",
            pretty_bytes(&json!({
                "origin": "real",
                "estimated_cost_covered_provider_roles": input.cost.estimated_cost_covered_provider_roles,
                "provider_charge_covered_provider_roles": input.cost.provider_charge_covered_provider_roles,
                "measured_duration_millis": input.cost.measured_duration_millis,
                "estimated_cost_microunits": input.cost.estimated_cost_microunits,
                "provider_charge_microunits": input.cost.provider_charge_microunits,
                "candidate_sha": candidate_sha,
                "provider_state_sha256": provider_state_sha256,
            })),
        ),
        (
            "privacy-permissions.json",
            pretty_bytes(&json!({
                "origin": "real",
                "permission_suite": input.privacy_permissions.permission_suite,
                "accepted_private_context_leakage": input.privacy_permissions.accepted_private_context_leakage,
                "accepted_false_owner_attribution": input.privacy_permissions.accepted_false_owner_attribution,
                "revocation": input.privacy_permissions.revocation,
                "egress_denial": input.privacy_permissions.egress_denial,
                "candidate_sha": candidate_sha,
                "provider_state_sha256": provider_state_sha256,
            })),
        ),
        (
            "human-evaluation.json",
            pretty_bytes(&json!({
                "origin": "real",
                "rubric_version": input.human_evaluation.rubric_version,
                "reviewer_count": input.human_evaluation.reviewer_count,
                "dimensions": input.human_evaluation.dimensions,
                "usable_for_continuation": input.human_evaluation.usable_for_continuation,
                "candidate_sha": candidate_sha,
                "provider_state_sha256": provider_state_sha256,
            })),
        ),
        (
            "known-limitations.md",
            format!(
                "RT0-Review-Status: {}\n\n{}\n",
                status_name(input.known_limitations.review_status),
                input.known_limitations.body.trim()
            )
            .into_bytes(),
        ),
    ]
}

fn status_name(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Passed => "passed",
        CheckStatus::Failed => "failed",
    }
}

fn pretty_bytes(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value).expect("manual supporting JSON serialization");
    bytes.push(b'\n');
    bytes
}

fn read(path: &Path) -> Result<Vec<u8>, i32> {
    fs::read(path).map_err(|_| fail("INPUT_INVALID"))
}

fn fail(code: &'static str) -> i32 {
    eprintln!("{{\"ok\":false,\"code\":\"{code}\"}}");
    2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> ManualSupportingInput {
        ManualSupportingInput {
            schema_version: INPUT_SCHEMA.into(),
            attestation: REQUIRED_ATTESTATION.into(),
            acceptance: AcceptanceInput {
                owner_happy: CheckStatus::Passed,
                visitor_happy: CheckStatus::Passed,
                correction: CheckStatus::Passed,
                failure_recovery: CheckStatus::Failed,
                revoke_deny: CheckStatus::Passed,
            },
            cost: CostInput {
                estimated_cost_covered_provider_roles: vec![
                    ProviderRole::Stt,
                    ProviderRole::Llm,
                    ProviderRole::Avatar,
                ],
                provider_charge_covered_provider_roles: vec![ProviderRole::Avatar],
                measured_duration_millis: 60_000,
                estimated_cost_microunits: Some(120),
                provider_charge_microunits: Some(80),
            },
            privacy_permissions: PrivacyInput {
                permission_suite: CheckStatus::Passed,
                accepted_private_context_leakage: 0,
                accepted_false_owner_attribution: 0,
                revocation: CheckStatus::Passed,
                egress_denial: CheckStatus::Passed,
            },
            human_evaluation: HumanInput {
                rubric_version: "rt0-human-0.1".into(),
                reviewer_count: 1,
                dimensions: HumanDimensions {
                    voice_similarity: RecordStatus::Recorded,
                    voice_naturalness: RecordStatus::Recorded,
                    appearance_plausibility: RecordStatus::Recorded,
                    persona_similarity: RecordStatus::Recorded,
                    conversation_naturalness: RecordStatus::Recorded,
                },
                usable_for_continuation: CheckStatus::Passed,
            },
            known_limitations: KnownLimitationsInput {
                review_status: CheckStatus::Failed,
                body: "One reviewed limitation remains.".into(),
            },
        }
    }

    #[test]
    fn capture_binds_real_claims_without_promoting_failed_observations() {
        let candidate = "a".repeat(40);
        let provider = "b".repeat(64);
        let artifacts = build_artifacts(&input(), &candidate, &provider);
        let acceptance: Value = serde_json::from_slice(
            &artifacts
                .iter()
                .find(|(name, _)| *name == "acceptance.json")
                .unwrap()
                .1,
        )
        .unwrap();
        assert_eq!(acceptance["origin"], "real");
        assert_eq!(acceptance["failure_recovery_path"], "failed");
        assert_eq!(acceptance["candidate_sha"], candidate);
        assert_eq!(acceptance["provider_state_sha256"], provider);
    }

    #[test]
    fn capture_requires_explicit_real_observation_attestation() {
        let mut value = input();
        value.attestation = "synthetic".into();
        assert_eq!(validate_input(&value), Err(2));
    }

    #[test]
    fn capture_rejects_embedded_limitations_status_marker() {
        let mut value = input();
        value.known_limitations.body = "RT0-Review-Status: passed\nnot allowed".into();
        assert_eq!(validate_input(&value), Err(2));
    }

    #[test]
    fn capture_rejects_duplicate_cost_provider_roles() {
        let mut value = input();
        value.cost.estimated_cost_covered_provider_roles =
            vec![ProviderRole::Stt, ProviderRole::Stt];
        assert_eq!(validate_input(&value), Err(2));
    }
}
