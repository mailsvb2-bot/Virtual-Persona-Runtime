from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "rt1-owner-control-center-0.1.json"
RT1_SPEC = ROOT / "docs" / "releases" / "RT1_RELEASE_SPEC.md"
CRATES = ROOT / "crates"

contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
spec = RT1_SPEC.read_text(encoding="utf-8")

if contract.get("schema") != "rt1-owner-control-center-spike-0.1":
    raise SystemExit("RT1 Owner Control Center spike schema drifted")
if contract.get("maturity") != "RESEARCH_REQUIRED":
    raise SystemExit("RT1 Owner Control Center spike must remain RESEARCH_REQUIRED")
for flag in (
    "production",
    "owner_control_center_runtime_implemented",
    "owner_control_center_ui_implemented",
    "durable_control_center_projection_implemented",
):
    if contract.get(flag) is not False:
        raise SystemExit(f"RT1 Owner Control Center spike must remain non-promoting: {flag}")

projection = contract.get("projection", {})
for invariant in (
    "is_projection_not_source_of_truth",
    "reads_canonical_claims_and_revision_history",
    "reads_canonical_provenance",
    "reads_canonical_audience_policy",
    "must_not_copy_provider_owned_truth",
):
    if projection.get(invariant) is not True:
        raise SystemExit(f"Owner Control Center projection invariant lost: {invariant}")

expected_classes = {
    "FACTS", "OPINIONS", "PREFERENCES", "STORIES", "BIOGRAPHY",
    "SKILLS", "RESTRICTIONS", "RELATIONSHIPS", "SYSTEM_INFERENCES"
}
if set(contract.get("content_classes", [])) != expected_classes:
    raise SystemExit("Owner Control Center content classes drifted")

actions = contract.get("owner_actions", {})
for action in ("confirm", "correct", "hide", "delete"):
    if actions.get(action, {}).get("routes_to") != "OWNER_CLAIM_LIFECYCLE":
        raise SystemExit(f"{action} must route to canonical owner claim lifecycle")
    if actions.get(action, {}).get("creates_second_claim_authority") is not False:
        raise SystemExit(f"{action} created a second claim authority")

restrict = actions.get("restrict_audience", {})
for key in ("may_only_narrow_visibility_in_this_flow", "must_invalidate_affected_public_projection"):
    if restrict.get(key) is not True:
        raise SystemExit(f"restrict-audience invariant lost: {key}")
if restrict.get("routes_to") != "CANONICAL_AUDIENCE_POLICY":
    raise SystemExit("restrict audience must route to canonical audience policy")

outdated = actions.get("mark_outdated", {})
for key in (
    "preserves_attributable_history",
    "excludes_from_current_factual_context",
    "must_invalidate_affected_derived_and_public_state",
    "does_not_mean_delete",
):
    if outdated.get(key) is not True:
        raise SystemExit(f"mark-outdated invariant lost: {key}")

reject = actions.get("reject_inference", {})
for key in (
    "applies_only_to_system_inference_or_derived_claim",
    "must_not_create_opposite_owner_claim",
    "must_not_upgrade_any_claim_to_owner_verified",
    "must_invalidate_affected_derived_and_public_state",
):
    if reject.get(key) is not True:
        raise SystemExit(f"reject-inference invariant lost: {key}")

provenance = actions.get("inspect_provenance", {})
if provenance.get("routes_to") != "CANONICAL_PROVENANCE_EVIDENCE":
    raise SystemExit("provenance inspection must route to canonical evidence")
for key in ("read_only", "must_respect_effective_disclosure", "must_not_expose_hidden_chain_of_thought"):
    if provenance.get(key) is not True:
        raise SystemExit(f"inspect-provenance invariant lost: {key}")

expected_hierarchy = [
    "EXPLICIT_OWNER_CORRECTION",
    "VERIFIED_OWNER_STATEMENT",
    "VERIFIED_SOURCE",
    "SYSTEM_INFERENCE",
    "UNVERIFIED_MODEL_KNOWLEDGE",
]
if contract.get("owner_correction_authority_hierarchy") != expected_hierarchy:
    raise SystemExit("owner correction authority hierarchy drifted")

for invariant in (
    "owner_actions_require_owner_authority",
    "visitor_cannot_mutate_control_center",
    "control_center_cannot_widen_authority",
    "control_center_cannot_bypass_claim_lifecycle",
    "control_center_cannot_bypass_evidence_disclosure",
    "stale_persona_version_mutation_fails_closed",
    "correction_outdated_rejection_and_restriction_propagate_to_affected_projections",
    "normal_owner_flow_does_not_require_provider_or_model_ids",
):
    if contract.get("invariants", {}).get(invariant) is not True:
        raise SystemExit(f"Owner Control Center invariant lost: {invariant}")

expected_fail_closed = {
    ("VISITOR_ATTEMPTS_CONTROL_CENTER_MUTATION", "DENY_BEFORE_PROVIDER_EGRESS"),
    ("RESTRICT_AUDIENCE_REQUEST_WOULD_WIDEN_VISIBILITY", "DENY_OR_ROUTE_TO_SEPARATE_REVIEWED_AUDIENCE_POLICY_CHANGE"),
    ("REJECT_INFERENCE_ATTEMPTS_TO_CREATE_OWNER_VERIFIED_OPPOSITE", "DENY"),
    ("PROVENANCE_VIEW_WOULD_DISCLOSE_RESTRICTED_SOURCE", "REDACT_OR_OMIT"),
    ("MARK_OUTDATED_WITH_STALE_PERSONA_VERSION", "DENY"),
}
actual_fail_closed = {
    (item.get("case"), item.get("decision"))
    for item in contract.get("fail_closed_examples", [])
}
if actual_fail_closed != expected_fail_closed:
    raise SystemExit("Owner Control Center fail-closed matrix drifted")

surface_suffixes = {".rs", ".ts", ".tsx", ".js", ".html"}
forbidden_markers = (
    "struct Rt1OwnerControlCenter",
    "OwnerControlCenterRecord",
    "/api/owner-control-center",
    'id="owner-control-center"',
)
for source in CRATES.rglob("*"):
    if not source.is_file() or source.suffix not in surface_suffixes:
        continue
    text = source.read_text(encoding="utf-8")
    marker = next((item for item in forbidden_markers if item in text), None)
    if marker is not None:
        raise SystemExit(
            "production RT1 Owner Control Center surface appeared while RT1 remains blocked "
            f"on RT0 exit: {source.relative_to(ROOT)} (marker={marker!r})"
        )

for required_spec_marker in (
    "Status:** `BLOCKED_ON_RT0_EXIT`",
    "Allowed pre-entry work:** `BOUNDED_FEASIBILITY_SPIKE_ONLY`",
    "Owner Control Center",
    "Owner Control Center feasibility boundary",
    "restrict audience",
    "mark outdated",
    "reject inference",
    "inspect provenance",
    "must remain contract-only and non-promoting",
):
    if required_spec_marker not in spec:
        raise SystemExit(f"Owner Control Center spike lost ReleaseSpec guard: {required_spec_marker}")

print("rt1-owner-control-center-spike: PASS")
