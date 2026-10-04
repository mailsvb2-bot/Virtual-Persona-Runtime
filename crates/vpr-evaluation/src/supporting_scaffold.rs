use serde_json::{Value, json};

pub const RT0_RUNTIME_SUPPORTING_FILES: [&str; 3] = [
    "owner-conversation.json",
    "visitor-conversation.json",
    "quality.json",
];

pub const RT0_MANUAL_SUPPORTING_FILES: [&str; 5] = [
    "acceptance.json",
    "cost.json",
    "privacy-permissions.json",
    "human-evaluation.json",
    "known-limitations.md",
];

#[must_use]
pub fn rt0_runtime_supporting_scaffold(
    candidate_sha: &str,
    provider_state_sha256: &str,
) -> Vec<(&'static str, String)> {
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
        ("owner-conversation.json", pretty(&conversation("owner"))),
        (
            "visitor-conversation.json",
            pretty(&conversation("visitor")),
        ),
        (
            "quality.json",
            pretty(&json!({
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
    ]
}

#[must_use]
pub fn rt0_manual_supporting_scaffold(
    candidate_sha: &str,
    provider_state_sha256: &str,
) -> Vec<(&'static str, String)> {
    vec![
        (
            "acceptance.json",
            pretty(&json!({
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
            "cost.json",
            pretty(&json!({
                "origin": "synthetic",
                "estimated_cost_covered_provider_roles": [],
                "provider_charge_covered_provider_roles": [],
                "measured_duration_millis": 0,
                "estimated_cost_microunits": Value::Null,
                "provider_charge_microunits": Value::Null,
                "candidate_sha": candidate_sha,
                "provider_state_sha256": provider_state_sha256,
            })),
        ),
        (
            "privacy-permissions.json",
            pretty(&json!({
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
            pretty(&json!({
                "origin": "synthetic",
                "rubric_version": "UNREVIEWED",
                "reviewer_count": 0,
                "owner_human_participant_verified": "failed",
                "visitor_distinct_non_owner_human_verified": "failed",
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
            "RT0-Review-Status: failed\n\nScaffold only. Replace this text with the reviewed known limitations for the exact candidate before any exit-gate attempt.\n".into(),
        ),
    ]
}

fn pretty(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(value).expect("JSON scaffold serialization");
    text.push('\n');
    text
}
