const APP: &str = include_str!("../ui/src/app.ts");
const SESSION_EVIDENCE: &str = include_str!("../../vpr-evaluation/src/session_evidence.rs");

#[test]
fn owner_lab_records_detailed_video_connection_stages() {
    for marker in [
        "\"transport_connect_started\"",
        "\"remote_video_track_received\"",
        "\"remote_video_attached\"",
        "\"video_ready\"",
        "\"end_to_end_video_ready\"",
    ] {
        assert!(
            APP.contains(marker),
            "missing RT0 video connection stage marker: {marker}"
        );
    }

    assert!(
        APP.contains(
            "queueConnectionMediaEvidence(\"video_ready\", now - connectEvidenceStartedAt)"
        ),
        "canonical prepared-media video timer must remain separate"
    );
    assert!(
        !APP.contains("await postMediaEvidence(\n      \"transport_connect_started\""),
        "connection telemetry must not block transport startup"
    );
    assert!(
        APP.contains("await flushConnectionMediaEvidence();"),
        "queued connection telemetry must flush before RT0 session closure"
    );
}

#[test]
fn aggregate_exports_video_connection_stage_distributions() {
    for marker in [
        "TransportConnectStarted",
        "RemoteVideoTrackReceived",
        "RemoteVideoAttached",
        "transport_connect_started",
        "remote_video_track_received",
        "remote_video_attached",
    ] {
        assert!(
            SESSION_EVIDENCE.contains(marker),
            "session evidence contract missing video stage: {marker}"
        );
    }
}
