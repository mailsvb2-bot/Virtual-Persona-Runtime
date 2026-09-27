from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "rt1-owner-claim-lifecycle-0.1.json"
RT1_SPEC = ROOT / "docs" / "releases" / "RT1_RELEASE_SPEC.md"
PROFILE = ROOT / "crates" / "vpr-domain" / "src" / "profile.rs"

contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
spec = RT1_SPEC.read_text(encoding="utf-8")
profile = PROFILE.read_text(encoding="utf-8")

if contract.get("schema") != "rt1-owner-claim-lifecycle-spike-0.1":
    raise SystemExit("RT1 owner-claim lifecycle spike schema drifted")
if contract.get("maturity") != "RESEARCH_REQUIRED":
    raise SystemExit("RT1 owner-claim lifecycle spike must remain RESEARCH_REQUIRED")
for flag in (
    "production",
    "domain_mutations_implemented",
    "durable_state_implemented",
    "public_projection_implemented",
):
    if contract.get(flag) is not False:
        raise SystemExit(f"RT1 owner-claim spike must remain non-promoting: {flag}")

reality = contract.get("current_rt0_reality", {})
expected_reality = {
    "approve_present": True,
    "correct_present": True,
    "hide_present": False,
    "delete_present": False,
    "production_claim_lifecycle_complete": False,
}
if reality != expected_reality:
    raise SystemExit("RT0 claim-lifecycle reality drifted")

for marker in (
    "pub fn approve(&mut self)",
    "pub fn correct(",
    "pub fn approve_claim(&mut self",
    "pub fn correct_claim(",
):
    if marker not in profile:
        raise SystemExit(f"expected RT0 claim capability is missing: {marker}")

for forbidden_marker in (
    "pub fn hide_claim(",
    "pub fn delete_claim(",
    "pub fn hide(&mut self)",
    "pub fn delete(&mut self)",
):
    if forbidden_marker in profile:
        raise SystemExit(
            "production claim mutation appeared while RT1 remains blocked on RT0 exit: "
            + forbidden_marker
        )

expected_states = [
    "PENDING_REVIEW",
    "CONFIRMED",
    "CORRECTED",
    "HIDDEN",
    "DELETED",
]
if contract.get("states") != expected_states:
    raise SystemExit("RT1 claim review state vocabulary drifted")

expected_transitions = {
    ("PENDING_REVIEW", "CONFIRMED"),
    ("PENDING_REVIEW", "CORRECTED"),
    ("PENDING_REVIEW", "HIDDEN"),
    ("PENDING_REVIEW", "DELETED"),
    ("CONFIRMED", "CORRECTED"),
    ("CONFIRMED", "HIDDEN"),
    ("CONFIRMED", "DELETED"),
    ("CORRECTED", "CORRECTED"),
    ("CORRECTED", "HIDDEN"),
    ("CORRECTED", "DELETED"),
    ("HIDDEN", "CORRECTED"),
    ("HIDDEN", "DELETED"),
}
actual_transitions = {tuple(item) for item in contract.get("transitions", [])}
if actual_transitions != expected_transitions:
    raise SystemExit("RT1 claim lifecycle transition contract drifted")

ops = contract.get("operation_semantics", {})
confirm = ops.get("confirm", {})
correct = ops.get("correct", {})
hide = ops.get("hide", {})
delete = ops.get("delete", {})

if confirm.get("requires_owner_authority") is not True:
    raise SystemExit("claim confirmation must require owner authority")
if confirm.get("may_widen_audience") is not False:
    raise SystemExit("claim confirmation must not silently widen audience")

for invariant in (
    "requires_owner_authority",
    "preserves_attributable_prior_revision",
    "advances_persona_version_after_initial_review",
    "invalidates_derived_and_public_state",
):
    if correct.get(invariant) is not True:
        raise SystemExit(f"claim correction invariant lost: {invariant}")

for invariant in (
    "requires_owner_authority",
    "retains_canonical_content",
    "removes_claim_from_normal_context_and_public_projection",
    "advances_persona_version_after_initial_review",
    "invalidates_derived_and_public_state",
):
    if hide.get(invariant) is not True:
        raise SystemExit(f"claim hide invariant lost: {invariant}")
if hide.get("is_delete") is not False:
    raise SystemExit("hide must remain semantically distinct from delete")

for invariant in (
    "requires_owner_authority",
    "removes_active_claim_content",
    "allows_minimal_non_content_tombstone",
    "advances_persona_version_after_initial_review",
    "invalidates_or_erases_derived_state",
    "must_not_resurrect_from_cache_or_restore",
):
    if delete.get(invariant) is not True:
        raise SystemExit(f"claim delete invariant lost: {invariant}")
if delete.get("retains_deleted_content_in_tombstone") is not False:
    raise SystemExit("claim deletion tombstone must not retain deleted content")

invariants = contract.get("invariants", {})
for required in (
    "claim_id_stable_across_correction_and_hide",
    "provider_identity_never_becomes_claim_identity",
    "owner_mutation_cannot_widen_authority",
    "stale_persona_version_must_fail_closed",
    "hidden_claim_not_disclosed_by_normal_context",
    "deleted_claim_not_disclosed",
    "deleted_content_not_preserved_merely_for_revision_history",
    "audit_may_preserve_non_content_deletion_evidence",
    "derived_state_must_not_outlive_effective_correction_hide_or_delete",
):
    if invariants.get(required) is not True:
        raise SystemExit(f"claim lifecycle invariant lost: {required}")

required_examples = {
    ("VISITOR_ATTEMPTS_OWNER_CLAIM_MUTATION", "DENY_BEFORE_PROVIDER_EGRESS"),
    ("STALE_PERSONA_VERSION_MUTATION", "DENY"),
    ("HIDDEN_CLAIM_IN_PUBLIC_CONTEXT", "DENY_DISCLOSURE"),
    ("DELETED_CLAIM_IN_ANY_ACTIVE_CONTEXT", "DENY_DISCLOSURE"),
    ("DELETE_WITH_DERIVED_STATE_NOT_INVALIDATED", "FAIL_CLOSED"),
}
actual_examples = {
    (item.get("case"), item.get("decision"))
    for item in contract.get("fail_closed_examples", [])
}
if actual_examples != required_examples:
    raise SystemExit("claim lifecycle fail-closed matrix drifted")

for required_spec_marker in (
    "Status:** `BLOCKED_ON_RT0_EXIT`",
    "Allowed pre-entry work:** `BOUNDED_FEASIBILITY_SPIKE_ONLY`",
    "PENDING_REVIEW -> CONFIRMED | CORRECTED | HIDDEN | DELETED",
    "owner correction/deletion invalidates affected derived/public state",
    "Claim lifecycle feasibility boundary",
    "must remain contract-only and non-promoting",
):
    if required_spec_marker not in spec:
        raise SystemExit(f"RT1 claim lifecycle spike lost ReleaseSpec guard: {required_spec_marker}")

print("rt1-owner-claim-lifecycle-spike: PASS")
