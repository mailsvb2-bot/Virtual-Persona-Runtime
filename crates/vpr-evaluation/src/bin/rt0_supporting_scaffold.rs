use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use vpr_evaluation::{ProviderStateManifest, RT0_PROVIDER_STATE_SCHEMA, sha256_hex};

const FILES: [&str; 10] = [
    "ci-evidence.json",
    "e2e-evidence.json",
    "owner-conversation.json",
    "visitor-conversation.json",
    "acceptance.json",
    "quality.json",
    "cost.json",
    "privacy-permissions.json",
    "human-evaluation.json",
    "known-limitations.md",
];

fn main() {
    if let Err(error) = run(std::env::args().skip(1).collect()) {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    if args.len() != 3 {
        return Err(
            "usage: vpr-rt0-supporting-scaffold <output-dir> <provider-state.json> <candidate-sha>"
                .into(),
        );
    }
    let output = PathBuf::from(&args[0]);
    let provider_state_path = PathBuf::from(&args[1]);
    let candidate_sha = args[2].trim();
    validate_candidate(candidate_sha)?;

    let provider_bytes =
        fs::read(&provider_state_path).map_err(|_| "provider-state file could not be read")?;
    let provider: ProviderStateManifest = serde_json::from_slice(&provider_bytes)
        .map_err(|_| "provider-state JSON is invalid")?;
    if provider.schema_version != RT0_PROVIDER_STATE_SCHEMA {
        return Err("provider-state schema is not supported".into());
    }
    let provider_state_sha256 = sha256_hex(&provider_bytes);

    if output.exists() && !output.is_dir() {
        return Err("output path exists and is not a directory".into());
    }
    fs::create_dir_all(&output).map_err(|_| "output directory could not be created")?;
    if FILES.iter().any(|name| output.join(name).exists()) {
        return Err("refusing to overwrite an existing canonical supporting-evidence file".into());
    }

    for (name, content) in scaffold(candidate_sha, &provider_state_sha256) {
        let path = output.join(name);
        fs::write(path, content).map_err(|_| "supporting-evidence scaffold write failed")?;
    }

    println!(
        "RT0 supporting scaffold created at {}\n         IMPORTANT: every real-evidence claim is intentionally synthetic/failed/missing until          replaced by reviewed observations from this exact candidate.",
        output.display()
    );
    Ok(())
}

fn scaffold(candidate_sha: &str, provider_state_sha256: &str) -> Vec<(&'static str, String)> {
    let automated = |status: &str| {
        json!({
            "status": status,
            "candidate_sha": candidate_sha,
        })
    };
    let conversation = |role: &str| {
        json!({
            "origin": "synthetic",
            "role": role,
            "russian": "failed",
            "voice": "failed",
            "video": "failed",
            "completed_turns": 0,
            "interruption_exercised": "failed",
            "candidate_sha": candidate_sha,
            "provider_state_sha256": provider_state_sha256,
        })
    };
    let latency = || json!({"samples": 1, "p50": 0, "p95": 0});

    vec![
        ("ci-evidence.json", pretty(automated("failed"))),
        ("e2e-evidence.json", pretty(automated("failed"))),
        ("owner-conversation.json", pretty(conversation("owner"))),
        ("visitor-conversation.json", pretty(conversation("visitor"))),
        (
            "acceptance.json",
            pretty(json!({
                "origin": "synthetic",
                "owner_happy_path": "failed",
                "visitor_happy_path": "failed",
                "correction_path": "failed",
                "failure_recovery_path": "failed",
                "revoke_deny_path": "failed",
                "candidate_sha": candidate_sha,
                "provider_state_sha256": provider_state_sha256,
            })),
        ),
        (
            "quality.json",
            pretty(json!({
                "origin": "synthetic",
                "text_first_meaningful_response": latency(),
                "first_meaningful_audio": latency(),
                "interruption_stop": latency(),
                "first_useful_video": latency(),
                "av_sync_absolute_offset": latency(),
                "recoverable_reconnect": latency(),
                "candidate_sha": candidate_sha,
                "provider_state_sha256": provider_state_sha256,
            })),
        ),
        (
            "cost.json",
            pretty(json!({
                "origin": "synthetic",
                "measured_duration_millis": 0,
                "measured_cost_microunits": Value::Null,
                "provider_charge_microunits": Value::Null,
                "candidate_sha": candidate_sha,
                "provider_state_sha256": provider_state_sha256,
            })),
        ),
        (
            "privacy-permissions.json",
            pretty(json!({
                "origin": "synthetic",
                "permission_suite": "failed",
                "accepted_private_context_leakage": 0,
                "accepted_false_owner_attribution": 0,
                "revocation": "failed",
                "egress_denial": "failed",
                "candidate_sha": candidate_sha,
                "provider_state_sha256": provider_state_sha256,
            })),
        ),
        (
            "human-evaluation.json",
            pretty(json!({
                "origin": "synthetic",
                "rubric_version": "UNREVIEWED",
                "reviewer_count": 0,
                "dimensions": {
                    "voice_similarity": "missing",
                    "voice_naturalness": "missing",
                    "appearance_plausibility": "missing",
                    "persona_similarity": "missing",
                    "conversation_naturalness": "missing",
                },
                "usable_for_continuation": "failed",
                "candidate_sha": candidate_sha,
                "provider_state_sha256": provider_state_sha256,
            })),
        ),
        (
            "known-limitations.md",
            "RT0-Review-Status: failed\n\n             Scaffold only. Replace this text with the reviewed known limitations for the exact              candidate before any exit-gate attempt.\n"
                .into(),
        ),
    ]
}

fn pretty(value: Value) -> String {
    let mut text = serde_json::to_string_pretty(&value).expect("JSON scaffold serialization");
    text.push('\n');
    text
}

fn validate_candidate(value: &str) -> Result<(), String> {
    if value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("candidate SHA must be exactly 40 hexadecimal characters".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaffold_is_bound_but_deliberately_non_promoting() {
        let candidate = "a".repeat(40);
        let provider = "b".repeat(64);
        let files = scaffold(&candidate, &provider);
        assert_eq!(files.len(), FILES.len());

        let owner: Value = serde_json::from_str(
            &files
                .iter()
                .find(|(name, _)| *name == "owner-conversation.json")
                .unwrap()
                .1,
        )
        .unwrap();
        assert_eq!(owner["candidate_sha"], candidate);
        assert_eq!(owner["provider_state_sha256"], provider);
        assert_eq!(owner["origin"], "synthetic");
        assert_eq!(owner["completed_turns"], 0);

        let human: Value = serde_json::from_str(
            &files
                .iter()
                .find(|(name, _)| *name == "human-evaluation.json")
                .unwrap()
                .1,
        )
        .unwrap();
        assert_eq!(human["reviewer_count"], 0);
        assert_eq!(human["dimensions"]["persona_similarity"], "missing");

        let limitations = &files
            .iter()
            .find(|(name, _)| *name == "known-limitations.md")
            .unwrap()
            .1;
        assert!(limitations.starts_with("RT0-Review-Status: failed"));
    }

    #[test]
    fn candidate_validation_is_fail_closed() {
        assert!(validate_candidate(&"a".repeat(40)).is_ok());
        assert!(validate_candidate("not-a-sha").is_err());
    }
}
