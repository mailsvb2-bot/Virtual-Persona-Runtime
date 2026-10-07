const APP: &str = include_str!("../ui/src/app.ts");

#[test]
fn automatic_terminal_paths_flush_connection_evidence_before_close() {
    assert!(
        APP.contains(
            "const connectionEvidenceError = rt0EvidenceMode\n    ? await tryFlushConnectionMediaEvidence()\n    : null;"
        ),
        "automatic LiveKit disconnect must attempt connection-evidence flush before sealing the session"
    );
    assert!(
        APP.contains(
            "const connectionEvidenceError = rt0EvidenceMode\n      ? await tryFlushConnectionMediaEvidence()\n      : null;"
        ),
        "connect failure cleanup must attempt connection-evidence flush before sealing the session"
    );
    assert!(
        APP.contains("connection evidence incomplete:"),
        "automatic terminal paths must surface incomplete connection evidence instead of silently omitting it"
    );
}

#[test]
fn revoke_is_never_blocked_by_connection_evidence_failure() {
    assert!(
        APP.contains("if (connectionEvidenceError && kind === \"close\")"),
        "explicit close may remain fail-closed on evidence failure"
    );
    assert!(
        !APP.contains("if (connectionEvidenceError && kind === \"revoke\")"),
        "revocation must not be blocked by telemetry failure"
    );
    assert!(
        APP.contains("await api<{ ok: true }>(`/api/session/${kind}`, {});"),
        "revoke must still reach the canonical session endpoint"
    );
}
