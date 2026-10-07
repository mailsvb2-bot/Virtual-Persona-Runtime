const APP: &str = include_str!("../ui/src/app.ts");
const SESSION_EVIDENCE: &str = include_str!("../../vpr-evaluation/src/session_evidence.rs");

#[test]
fn owner_lab_records_user_to_avatar_connection_stages_without_redefining_video_ready() {
    for marker in [
        "connectJourneyStartedAt = performance.now()",
        "\"backend_start_ready\"",
        "\"transport_connected\"",
        "\"end_to_end_video_ready\"",
        "queueConnectionMediaEvidence(\"video_ready\", now - connectEvidenceStartedAt)",
    ] {
        assert!(
            APP.contains(marker),
            "missing RT0 connection timing marker: {marker}"
        );
    }

    assert!(
        APP.contains("connectEvidenceStartedAt = performance.now()"),
        "prepared-media-path video timer must remain distinct from user journey timing"
    );
}

#[test]
fn connection_stage_events_are_exported_and_aggregated_separately() {
    for marker in [
        "BackendStartReady",
        "TransportConnected",
        "EndToEndVideoReady",
        "backend_start_ready",
        "transport_connected",
        "end_to_end_first_useful_video",
    ] {
        assert!(
            SESSION_EVIDENCE.contains(marker),
            "session evidence contract missing connection stage: {marker}"
        );
    }
}
