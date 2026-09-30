use serde_json::{Value, json};

pub const RT0_RUNTIME_SUPPORTING_FILES: [&str; 3] = [
    "owner-conversation.json",
    "visitor-conversation.json",
    "quality.json",
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

fn pretty(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(value).expect("JSON scaffold serialization");
    text.push('\n');
    text
}
