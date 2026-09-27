from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "rt1-behavior-training-0.1.json"
RT1_SPEC = ROOT / "docs" / "releases" / "RT1_RELEASE_SPEC.md"
CRATES = ROOT / "crates"

contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
spec = RT1_SPEC.read_text(encoding="utf-8")

if contract.get("schema") != "rt1-behavior-training-spike-0.1":
    raise SystemExit("RT1 behavior-training spike schema drifted")
if contract.get("maturity") != "RESEARCH_REQUIRED":
    raise SystemExit("RT1 behavior-training spike must remain RESEARCH_REQUIRED")
for flag in (
    "production",
    "behavior_runtime_implemented",
    "durable_behavior_state_implemented",
    "owner_ui_implemented",
):
    if contract.get(flag) is not False:
        raise SystemExit(f"RT1 behavior-training spike must remain non-promoting: {flag}")

expected_flow = [
    "QUESTION",
    "PERSONA_ANSWER",
    "OWNER_FEEDBACK",
    "BEHAVIOR_CHANGE_PROPOSAL",
    "BEFORE_AFTER_PREVIEW",
    "OWNER_APPROVAL",
    "NEW_PERSONA_VERSION",
]
if contract.get("canonical_flow") != expected_flow:
    raise SystemExit("behavior-training canonical flow drifted")

expected_feedback = {
    "TOO_FORMAL",
    "TOO_INFORMAL",
    "TOO_LONG",
    "TOO_SHORT",
    "NOT_MY_WORDING",
    "WRONG_TONE",
    "WRONG_FACT",
    "WRONG_BOUNDARY",
    "DO_NOT_ATTRIBUTE_THIS_OPINION",
    "PREFERRED_ANSWER",
}
if set(contract.get("feedback_kinds", [])) != expected_feedback:
    raise SystemExit("behavior-training feedback vocabulary drifted")

if set(contract.get("preferred_answer_input", [])) != {"TEXT", "VOICE"}:
    raise SystemExit("preferred answer input must preserve text + voice")

proposal = contract.get("proposal_invariants", {})
for invariant in (
    "binds_exact_base_persona_version",
    "active_persona_unchanged_before_explicit_owner_approval",
    "proposal_is_not_active_persona_state",
    "preview_uses_candidate_behavior_only",
    "preview_requires_several_examples",
    "rejection_or_abandonment_leaves_active_persona_unchanged",
):
    if proposal.get(invariant) is not True:
        raise SystemExit(f"behavior proposal invariant lost: {invariant}")

routing = contract.get("safety_routing", {})
for invariant in (
    "style_feedback_may_form_behavior_candidate",
    "wrong_fact_must_not_bypass_owner_claim_correction_authority",
    "wrong_boundary_must_not_widen_policy_or_consent",
    "do_not_attribute_opinion_must_not_create_verified_owner_belief",
    "preferred_answer_must_not_silently_become_verified_fact_or_opinion",
):
    if routing.get(invariant) is not True:
        raise SystemExit(f"behavior feedback safety routing lost: {invariant}")

approval = contract.get("approval_invariants", {})
for invariant in (
    "requires_owner_authority",
    "requires_exact_base_persona_version_match",
    "stale_base_version_fails_closed",
    "creates_new_persona_version",
    "does_not_mutate_prior_persona_version_in_place",
    "must_be_auditable",
    "must_be_idempotent_when_retried",
    "concurrent_newer_correction_or_revocation_must_win",
):
    if approval.get(invariant) is not True:
        raise SystemExit(f"behavior approval invariant lost: {invariant}")

expected_forbidden = {
    "SILENT_ACTIVE_PERSONA_MUTATION_FROM_FEEDBACK",
    "SILENT_PERMISSION_EXPANSION",
    "SILENT_CONSENT_EXPANSION",
    "SILENT_DELEGATION_EXPANSION",
    "SILENT_OWNER_BELIEF_ATTRIBUTION",
    "PROVIDER_IDENTITY_AS_BEHAVIOR_IDENTITY",
}
if set(contract.get("forbidden_side_effects", [])) != expected_forbidden:
    raise SystemExit("behavior-training forbidden-side-effect contract drifted")

required_examples = {
    ("VISITOR_SUBMITS_BEHAVIOR_APPROVAL", "DENY_BEFORE_PROVIDER_EGRESS"),
    ("OWNER_APPROVES_PROPOSAL_FOR_STALE_PERSONA_VERSION", "DENY"),
    ("FEEDBACK_ATTEMPTS_TO_WIDEN_BOUNDARY", "ROUTE_TO_CANONICAL_POLICY_REVIEW_OR_DENY"),
    ("WRONG_FACT_FEEDBACK_ATTEMPTS_DIRECT_BEHAVIOR_INJECTION", "ROUTE_TO_CANONICAL_CLAIM_CORRECTION"),
    ("PREVIEW_NOT_COMPLETED", "DENY_APPROVAL"),
}
actual_examples = {
    (item.get("case"), item.get("decision"))
    for item in contract.get("fail_closed_examples", [])
}
if actual_examples != required_examples:
    raise SystemExit("behavior-training fail-closed matrix drifted")

# RT1 is still blocked on RT0 exit: this spike must not accidentally introduce
# a production BehaviorChangeProposal implementation into Rust crates.
for source in CRATES.rglob("*.rs"):
    text = source.read_text(encoding="utf-8")
    if "BehaviorChangeProposal" in text:
        raise SystemExit(
            "production BehaviorChangeProposal appeared while RT1 remains blocked on RT0 exit: "
            + str(source.relative_to(ROOT))
        )

for required_spec_marker in (
    "Status:** `BLOCKED_ON_RT0_EXIT`",
    "Allowed pre-entry work:** `BOUNDED_FEASIBILITY_SPIKE_ONLY`",
    "behavior training by example",
    "Behavior training feasibility boundary",
    "Question -> Persona answer -> Owner feedback -> BehaviorChangeProposal",
    "active Persona remains unchanged until explicit owner approval",
    "must remain contract-only and non-promoting",
):
    if required_spec_marker not in spec:
        raise SystemExit(f"RT1 behavior-training spike lost ReleaseSpec guard: {required_spec_marker}")

print("rt1-behavior-training-spike: PASS")
