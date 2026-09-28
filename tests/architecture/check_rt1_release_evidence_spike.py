from __future__ import annotations
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "rt1-release-evidence-0.1.json"
SPEC = ROOT / "docs" / "releases" / "RT1_RELEASE_SPEC.md"
CRATES = ROOT / "crates"

c = json.loads(CONTRACT.read_text(encoding="utf-8"))
spec = SPEC.read_text(encoding="utf-8")

if c.get("schema") != "rt1-release-evidence-spike-0.1":
    raise SystemExit("RT1 release-evidence schema drifted")
if c.get("maturity") != "RESEARCH_REQUIRED":
    raise SystemExit("RT1 release-evidence spike must remain RESEARCH_REQUIRED")
for flag in ("production","rt1_exit_gate_implemented","release_evidence_bundle_implemented","human_acceptance_completed"):
    if c.get(flag) is not False:
        raise SystemExit(f"RT1 release-evidence spike must remain non-promoting: {flag}")

for key in (
    "git_candidate_required",
    "release_spec_version_or_digest_required",
    "schema_or_migration_version_required",
    "provider_model_representation_state_required",
    "material_candidate_change_invalidates_prior_evidence",
):
    if c.get("exact_candidate_binding",{}).get(key) is not True:
        raise SystemExit(f"exact-candidate invariant lost: {key}")

expected = {
    "UNIT_CONTRACT_INTEGRATION_BROWSER_E2E",
    "FULL_OWNER_JOURNEY",
    "INDEPENDENT_VISITOR_REAL_CONVERSATION",
    "CORRECTION_PATH",
    "PREPARATION_FAILURE_RECOVERY_PATH",
    "PAUSE_UNPUBLISH_DENY_PATH",
    "QUALITY_LATENCY_MEASUREMENTS",
    "COST_EVIDENCE",
    "PRIVACY_PERMISSION_RESULTS",
    "KNOWN_LIMITATIONS",
    "ROLLBACK_OR_SAFE_ROLLFORWARD_PROOF",
}
if set(c.get("mandatory_evidence",[])) != expected:
    raise SystemExit("mandatory RT1 evidence set drifted")

q = c.get("quality_contract",{})
for key in (
    "owner_journey_time_measured",
    "publish_completion_success_rate_measured",
    "visitor_bootstrap_and_first_meaningful_response_latency_measured",
    "text_voice_video_readiness_and_retry_recovery_measured",
    "pause_unpublish_propagation_measured",
    "cost_estimate_available_before_publication_or_use",
    "numeric_thresholds_beyond_inherited_rt0_require_measured_calibration_before_promotion",
):
    if q.get(key) is not True:
        raise SystemExit(f"RT1 quality invariant lost: {key}")
if q.get("accepted_private_context_leakage") != 0:
    raise SystemExit("private-context leakage acceptance must remain zero")
if q.get("accepted_false_owner_opinion_attribution") != 0:
    raise SystemExit("false owner-opinion attribution acceptance must remain zero")

for key in (
    "deterministic_tests_do_not_substitute_for_real_provider_evidence",
    "synthetic_or_mock_success_does_not_prove_real_visitor_conversation",
    "missing_provider_charge_not_invented",
    "missing_human_review_not_inferred_from_ci",
    "rt1_must_not_claim_production_ready_before_complete_reviewed_bundle",
):
    if c.get("anti_fabrication",{}).get(key) is not True:
        raise SystemExit(f"RT1 evidence anti-fabrication invariant lost: {key}")

expected_fail = {
    ("CI_GREEN_BUT_REAL_VISITOR_VIDEO_EVIDENCE_MISSING","RT1_EXIT_NOT_PASSED"),
    ("EVIDENCE_FROM_DIFFERENT_GIT_CANDIDATE","STALE_EVIDENCE_REJECTED"),
    ("PROVIDER_CONFIGURATION_MATERIALLY_CHANGED_AFTER_MEASUREMENT","REMEASURE_AFFECTED_EVIDENCE"),
    ("PRIVATE_CONTEXT_LEAKAGE_OBSERVED","RT1_EXIT_NOT_PASSED"),
    ("FALSE_OWNER_OPINION_ATTRIBUTION_OBSERVED","RT1_EXIT_NOT_PASSED"),
}
actual_fail = {(x.get("case"),x.get("decision")) for x in c.get("fail_closed_examples",[])}
if actual_fail != expected_fail:
    raise SystemExit("RT1 release-evidence fail-closed matrix drifted")

forbidden = (
    "struct Rt1ExitGate",
    "Rt1ProductionReady",
    "/api/rt1/release-evidence",
)
for source in CRATES.rglob("*"):
    if not source.is_file() or source.suffix not in {".rs",".ts",".tsx",".js",".html"}:
        continue
    txt = source.read_text(encoding="utf-8")
    marker = next((m for m in forbidden if m in txt),None)
    if marker:
        raise SystemExit(f"production RT1 release-evidence surface appeared before RT1 entry: {source.relative_to(ROOT)} ({marker})")

for marker in (
    "Status:** `BLOCKED_ON_RT0_EXIT`",
    "RT1 ReleaseEvidence must bind at least:",
    "RT1 Quality / ReleaseEvidence feasibility boundary",
    "CI success alone does not satisfy the RT1 exit gate",
    "must remain contract-only and non-promoting",
):
    if marker not in spec:
        raise SystemExit(f"RT1 release-evidence spike lost ReleaseSpec guard: {marker}")

print("rt1-release-evidence-spike: PASS")
