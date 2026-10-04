const RUNBOOK: &str = include_str!("../../../docs/release-evidence/rt0/RUNBOOK.md");

#[test]
fn rt0_runbook_contains_the_canonical_exit_toolchain_in_order() {
    let markers = [
        "vpr-rt0-evidence-prepare",
        "vpr-live-proof -- doctor",
        "vpr-live-proof -- candidate-bundle",
        "vpr-live-proof -- candidate-bundle-extract",
        "vpr-rt0-supporting-scaffold",
        "vpr-owner-lab -- --allow-egress",
        "vpr-rt0-session-aggregate",
        "vpr-rt0-runtime-readiness",
        "vpr-rt0-runtime-supporting",
        "vpr-rt0-automation-capture",
        "vpr-rt0-supporting-input-prefill",
        "vpr-rt0-supporting-capture",
        "vpr-rt0-supporting-preflight",
        "vpr-rt0-owner-golden",
        "vpr-rt0-exit-assemble",
        "vpr-rt0-evidence-inventory",
        "vpr-rt0-exit-evidence",
    ];

    let mut previous = 0;
    for marker in markers {
        let position = RUNBOOK
            .find(marker)
            .unwrap_or_else(|| panic!("RT0 runbook is missing canonical marker: {marker}"));
        assert!(
            position >= previous,
            "RT0 runbook order drifted at canonical marker: {marker}"
        );
        previous = position;
    }
}

#[test]
fn rt0_runbook_preserves_non_promoting_and_final_gate_markers() {
    for marker in [
        "REVIEW_REQUIRED",
        "reviewed_real_observations",
        "reviewed_exact_candidate_automation",
        "release_ready_claimed=false",
        "inventory_complete=true",
        "ready=true",
        "distinct real non-owner human",
        "Start again from this step",
    ] {
        assert!(
            RUNBOOK.contains(marker),
            "RT0 runbook is missing fail-closed marker: {marker}"
        );
    }
}

#[test]
fn rt0_runbook_forbids_manual_edits_of_canonical_generated_artifacts() {
    assert!(RUNBOOK.contains("no hand-edited canonical generated artifacts"));
    assert!(RUNBOOK.contains("Do not edit the canonical scaffold files by hand"));
}
