const APP: &str = include_str!("../ui/src/app.ts");

fn function_slice(start_marker: &str, end_marker: &str) -> &'static str {
    let start = APP
        .find(start_marker)
        .unwrap_or_else(|| panic!("missing function marker: {start_marker}"));
    let remainder = &APP[start..];
    let end = remainder
        .find(end_marker)
        .unwrap_or_else(|| panic!("missing next function marker: {end_marker}"));
    &remainder[..end]
}

fn assert_flush_before_close(label: &str, body: &str) {
    let flush = body
        .find("await tryFlushConnectionMediaEvidence()")
        .unwrap_or_else(|| panic!("{label}: missing connection-evidence flush"));
    let close = body
        .find("await api<{ ok: true }>(\"/api/session/close\", {})")
        .unwrap_or_else(|| panic!("{label}: missing canonical backend close"));
    assert!(
        flush < close,
        "{label}: connection evidence must be drained before the backend seals the session"
    );
}

#[test]
fn automatic_terminal_paths_flush_connection_evidence_before_close() {
    let unexpected_disconnect = function_slice(
        "const handleUnexpectedLiveKitDisconnect = async",
        "const connectWebRtcTransport = async",
    );
    assert_flush_before_close("unexpected LiveKit disconnect", unexpected_disconnect);
    assert!(
        unexpected_disconnect.contains("connection evidence incomplete:"),
        "unexpected disconnect must surface incomplete connection evidence"
    );

    let connect_failure = function_slice(
        "const connectAvatar = async",
        "const resetMicrophoneUpload =",
    );
    assert_flush_before_close("connect failure cleanup", connect_failure);
    assert!(
        connect_failure.contains("connection evidence incomplete:"),
        "connect failure cleanup must surface incomplete connection evidence"
    );
}

#[test]
fn revoke_is_never_blocked_by_connection_evidence_failure() {
    assert!(
        APP.contains("if (rt0EvidenceMode && connectionEvidenceError && kind === \"close\")"),
        "explicit close may remain fail-closed on evidence failure"
    );
    assert!(
        !APP.contains("if (rt0EvidenceMode && connectionEvidenceError && kind === \"revoke\")"),
        "revocation must not be blocked by telemetry failure"
    );
    assert!(
        APP.contains(r"await api<{ ok: true }>(`/api/session/${kind}`, {});"),
        "revoke must still reach the canonical session endpoint"
    );
}

#[test]
fn connection_evidence_drain_reaches_quiescence_and_runs_outside_rt0_mode() {
    assert!(
        APP.contains("const observedTail = connectionEvidenceTail;")
            && APP.contains("if (observedTail === connectionEvidenceTail) break;"),
        "flush must continue until no successor evidence write was appended while awaiting"
    );
    assert!(
        !APP.contains("rt0EvidenceMode\n    ? await tryFlushConnectionMediaEvidence()"),
        "terminal paths must drain connection evidence in ordinary Owner Lab sessions too"
    );
}
