use serde_json::json;
use vpr_evaluation::{
    LabSessionBindingError, RT0_OWNER_LAB_SESSION_BINDING_SCHEMA, bind_owner_lab_session_evidence,
    sha256_hex,
};

fn provider_state() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema_version":"rt0-provider-state-0.1",
        "providers":[
            {
                "role":"stt",
                "provider":"openai-transcription",
                "model_or_representation":"stt-model",
                "configuration_fingerprint_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            },
            {
                "role":"llm",
                "provider":"anthropic",
                "model_or_representation":"llm-model",
                "configuration_fingerprint_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            },
            {
                "role":"avatar",
                "provider":"did-agents-streams",
                "model_or_representation":"avatar-representation",
                "configuration_fingerprint_sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
            }
        ]
    }))
    .unwrap()
}

fn snapshot(session: u64, request: u64, elapsed: u64) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema_version":"rt0-owner-lab-session-evidence-0.2",
        "scope":"browser_observed_media_plane_only",
        "session_sequence":session,
        "canonical_playback_proven":true,
        "av_sync_proven":false,
        "voice_attempts":[{
            "request_sequence":request,
            "canonical_turn_sequence":request + 100,
            "canonical_output_sequence":request + 200,
            "canonical_playback_confirmed":true,
            "status":"completed",
            "failure_code":null,
            "stt_millis":100,
            "llm_millis":200,
            "avatar_millis":50,
            "server_total_millis":350,
            "stt_usage":{"input_units":10,"output_units":null,"estimated_cost_microunits":2,"provider_charge_microunits":3},
            "llm_usage":{"input_units":10,"output_units":4,"estimated_cost_microunits":5,"provider_charge_microunits":7}
        }],
        "media_events":[
            {"request_sequence":request,"kind":"audio_started","elapsed_millis":elapsed},
            {"request_sequence":null,"kind":"video_ready","elapsed_millis":elapsed + 100}
        ]
    }))
    .unwrap()
}

#[test]
fn binding_covers_exact_candidate_provider_state_and_raw_snapshot_bytes() {
    let provider_state = provider_state();
    let one = snapshot(1, 1, 400);
    let two = snapshot(2, 1, 600);
    let bound = bind_owner_lab_session_evidence(
        &[one.as_slice(), two.as_slice()],
        &provider_state,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .unwrap();

    assert_eq!(bound.schema_version, RT0_OWNER_LAB_SESSION_BINDING_SCHEMA);
    assert_eq!(
        bound.candidate_sha,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(bound.provider_state_sha256, sha256_hex(&provider_state));
    assert_eq!(
        bound.snapshot_sha256,
        vec![sha256_hex(&one), sha256_hex(&two)]
    );
    assert_eq!(bound.aggregate.sessions, 2);
    assert_eq!(bound.aggregate.first_meaningful_audio.unwrap().p95, 600);
    assert!(bound.aggregate.canonical_playback_proven);
    assert!(!bound.aggregate.av_sync_proven);
}

#[test]
fn binding_rejects_invalid_candidate_provider_state_and_snapshot_bytes() {
    let provider_state = provider_state();
    let one = snapshot(1, 1, 400);
    assert_eq!(
        bind_owner_lab_session_evidence(&[one.as_slice()], &provider_state, "not-a-sha"),
        Err(LabSessionBindingError::InvalidCandidateSha)
    );

    let invalid_provider = br#"{"schema_version":"rt0-provider-state-0.1","providers":[]}"#;
    assert_eq!(
        bind_owner_lab_session_evidence(
            &[one.as_slice()],
            invalid_provider,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ),
        Err(LabSessionBindingError::InvalidProviderState)
    );

    assert_eq!(
        bind_owner_lab_session_evidence(
            &[&b"not-json"[..]],
            &provider_state,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ),
        Err(LabSessionBindingError::InvalidSnapshot)
    );
}

#[test]
fn binding_rejects_duplicate_raw_artifacts_before_aggregation() {
    let provider_state = provider_state();
    let one = snapshot(1, 1, 400);
    assert_eq!(
        bind_owner_lab_session_evidence(
            &[one.as_slice(), one.as_slice()],
            &provider_state,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ),
        Err(LabSessionBindingError::DuplicateArtifact)
    );
}
