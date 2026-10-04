use std::{env, fs, path::Path, process};

use serde::{Deserialize, Serialize};
use vpr_evaluation::{CheckStatus, sha256_hex, validate_candidate_sha};

#[path = "rt0_automation_capture/transaction.rs"]
mod transaction;
use transaction::{AutomationCaptureTransaction, CaptureOutcome};

const INPUT_SCHEMA: &str = "rt0-automation-observations-0.1";
const RECEIPT_SCHEMA: &str = "rt0-automation-supporting-capture-receipt-0.1";
const REQUIRED_ATTESTATION: &str = "reviewed_exact_candidate_automation";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AutomationInput {
    schema_version: String,
    candidate_sha: String,
    attestation: String,
    ci: AutomationObservation,
    e2e: AutomationObservation,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AutomationObservation {
    status: CheckStatus,
    evidence_reference: String,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct AutomatedSupportingClaim<'a> {
    status: CheckStatus,
    candidate_sha: &'a str,
    evidence_reference_sha256: &'a str,
}

#[derive(Debug, Serialize)]
struct CaptureReceipt<'a> {
    schema_version: &'static str,
    candidate_sha: &'a str,
    input_sha256: String,
    ci_status: CheckStatus,
    e2e_status: CheckStatus,
    ci_evidence_reference_sha256: String,
    e2e_evidence_reference_sha256: String,
    already_committed: bool,
}

fn main() {
    if let Err(code) = run(&env::args().skip(1).collect::<Vec<_>>()) {
        process::exit(code);
    }
}

fn run(args: &[String]) -> Result<(), i32> {
    let [input_path, supporting_dir, candidate_sha] = args else {
        eprintln!(
            "usage: vpr-rt0-automation-capture <reviewed-automation.json> <supporting-dir> <exact-candidate-sha>"
        );
        return Err(2);
    };

    validate_candidate_sha(candidate_sha).map_err(|_| fail("INVALID_CANDIDATE"))?;
    let input_bytes = fs::read(input_path).map_err(|_| fail("INPUT_READ_FAILED"))?;
    let input: AutomationInput =
        serde_json::from_slice(&input_bytes).map_err(|_| fail("INVALID_INPUT"))?;
    validate_input(&input, candidate_sha)?;

    let root = Path::new(supporting_dir);
    if !root.is_dir() {
        return Err(fail("SUPPORTING_DIR_INVALID"));
    }

    let artifacts = build_artifacts(&input, candidate_sha)?;
    let scaffold = automation_scaffold(candidate_sha)?;
    let transaction = AutomationCaptureTransaction::new(
        root,
        candidate_sha,
        &input_bytes,
        &scaffold,
        &artifacts,
    );
    let outcome = transaction.commit()?;

    let receipt = CaptureReceipt {
        schema_version: RECEIPT_SCHEMA,
        candidate_sha,
        input_sha256: sha256_hex(&input_bytes),
        ci_status: input.ci.status,
        e2e_status: input.e2e.status,
        ci_evidence_reference_sha256: sha256_hex(input.ci.evidence_reference.trim().as_bytes()),
        e2e_evidence_reference_sha256: sha256_hex(input.e2e.evidence_reference.trim().as_bytes()),
        already_committed: outcome == CaptureOutcome::AlreadyCommitted,
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&receipt).map_err(|_| fail("OUTPUT_SERIALIZATION_FAILED"))?
    );
    Ok(())
}

fn validate_input(input: &AutomationInput, candidate_sha: &str) -> Result<(), i32> {
    if input.schema_version != INPUT_SCHEMA
        || input.attestation != REQUIRED_ATTESTATION
        || input.candidate_sha != candidate_sha
        || input.ci.evidence_reference.trim().is_empty()
        || input.e2e.evidence_reference.trim().is_empty()
    {
        return Err(fail("INVALID_INPUT"));
    }
    Ok(())
}

fn build_artifacts(
    input: &AutomationInput,
    candidate_sha: &str,
) -> Result<Vec<(&'static str, Vec<u8>)>, i32> {
    let ci_reference_sha256 = sha256_hex(input.ci.evidence_reference.trim().as_bytes());
    let e2e_reference_sha256 = sha256_hex(input.e2e.evidence_reference.trim().as_bytes());
    Ok(vec![
        (
            "ci-evidence.json",
            pretty_bytes(&AutomatedSupportingClaim {
                status: input.ci.status,
                candidate_sha,
                evidence_reference_sha256: &ci_reference_sha256,
            })?,
        ),
        (
            "e2e-evidence.json",
            pretty_bytes(&AutomatedSupportingClaim {
                status: input.e2e.status,
                candidate_sha,
                evidence_reference_sha256: &e2e_reference_sha256,
            })?,
        ),
    ])
}

fn automation_scaffold(
    candidate_sha: &str,
) -> Result<Vec<(&'static str, Vec<u8>)>, i32> {
    let scaffold = |status: CheckStatus| {
        serde_json::json!({
            "status": status,
            "candidate_sha": candidate_sha,
        })
    };
    Ok(vec![
        ("ci-evidence.json", pretty_bytes(&scaffold(CheckStatus::Failed))?),
        ("e2e-evidence.json", pretty_bytes(&scaffold(CheckStatus::Failed))?),
    ])
}

fn pretty_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, i32> {
    let mut bytes =
        serde_json::to_vec_pretty(value).map_err(|_| fail("OUTPUT_SERIALIZATION_FAILED"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn fail(code: &'static str) -> i32 {
    eprintln!("{{\"ok\":false,\"code\":\"{code}\"}}");
    2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> AutomationInput {
        AutomationInput {
            schema_version: INPUT_SCHEMA.into(),
            candidate_sha: "a".repeat(40),
            attestation: REQUIRED_ATTESTATION.into(),
            ci: AutomationObservation {
                status: CheckStatus::Passed,
                evidence_reference: "github-actions:ci:123".into(),
            },
            e2e: AutomationObservation {
                status: CheckStatus::Passed,
                evidence_reference: "github-actions:e2e:456".into(),
            },
        }
    }

    #[test]
    fn input_requires_exact_candidate_and_review_attestation() {
        let reviewed = input();
        assert!(validate_input(&reviewed, &"a".repeat(40)).is_ok());
        assert!(validate_input(&reviewed, &"b".repeat(40)).is_err());

        let mut unreviewed = input();
        unreviewed.attestation = "REVIEW_REQUIRED".into();
        assert!(validate_input(&unreviewed, &"a".repeat(40)).is_err());
    }

    #[test]
    fn generated_supporting_claims_do_not_embed_external_references() {
        let reviewed = input();
        let artifacts = build_artifacts(&reviewed, &reviewed.candidate_sha).unwrap();
        for (_, bytes) in artifacts {
            let text = String::from_utf8(bytes).unwrap();
            assert!(!text.contains("github-actions:"));
            assert!(text.contains("\"candidate_sha\""));
            assert!(text.contains("\"status\": \"passed\""));
        }
    }
}
