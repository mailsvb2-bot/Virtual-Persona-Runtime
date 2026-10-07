const APP: &str = include_str!("../ui/src/app.ts");

#[test]
fn automatic_terminal_paths_flush_connection_evidence_before_close() {
    assert!(
        APP.contains("const connectionEvidenceError = await tryFlushConnectionMediaEvidence();"),
        "automatic LiveKit disconnect must attempt connection-evidence flush before sealing the session"
    );
    assert!(
        APP.contains("const connectionEvidenceError = await tryFlushConnectionMediaEvidence();"),
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
        APP.contains("if (rt0EvidenceMode && connectionEvidenceError && kind === \"close\")"),
        "explicit close may remain fail-closed on evidence failure"
    );
    assert!(
        !APP.contains("if (rt0EvidenceMode && connectionEvidenceError && kind === \"revoke\")"),
        "revocation must not be blocked by telemetry failure"
    );
    assert!(
        APP.contains("await api<{ ok: true }>(`/api/session/${kind}`, {});"),
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
