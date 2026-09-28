from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "rt1-cost-budget-0.1.json"
RT1_SPEC = ROOT / "docs" / "releases" / "RT1_RELEASE_SPEC.md"
CRATES = ROOT / "crates"

contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
spec = RT1_SPEC.read_text(encoding="utf-8")

if contract.get("schema") != "rt1-cost-budget-spike-0.1":
    raise SystemExit("RT1 cost/budget spike schema drifted")
if contract.get("maturity") != "RESEARCH_REQUIRED":
    raise SystemExit("RT1 cost/budget spike must remain RESEARCH_REQUIRED")
for flag in (
    "production",
    "prepublication_estimate_runtime_implemented",
    "hard_session_budget_runtime_implemented",
    "owner_cost_ui_implemented",
):
    if contract.get(flag) is not False:
        raise SystemExit(f"RT1 cost/budget spike must remain non-promoting: {flag}")

expected_classes = {
    "PREPARATION_COST",
    "ESTIMATED_SESSION_COST",
    "LIVE_ESTIMATED_USAGE",
    "CONFIRMED_PROVIDER_CHARGE",
    "CUSTOMER_CHARGE",
    "MONTHLY_USAGE",
}
if set(contract.get("cost_classes", [])) != expected_classes:
    raise SystemExit("RT1 cost classes drifted")

for invariant in (
    "estimate_not_usage",
    "usage_not_provider_charge",
    "provider_charge_not_customer_charge",
    "missing_provider_charge_not_invented",
    "customer_charge_requires_explicit_business_contract",
):
    if contract.get("separation_invariants", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 cost separation invariant lost: {invariant}")

for invariant in (
    "required_before_publication_or_first_chargeable_use",
    "binds_supported_candidate_configuration",
    "records_estimate_basis_or_version",
    "stale_when_material_provider_model_or_representation_changes",
    "unknown_components_remain_unknown",
):
    if contract.get("prepublication_estimate", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 prepublication estimate invariant lost: {invariant}")

budget = contract.get("hard_session_budget", {})
if budget.get("must_not_be_silently_exceeded") is not True:
    raise SystemExit("hard session budget may not be silently exceeded")
if budget.get("exhaustion_reason_code") != "BUDGET_EXHAUSTED":
    raise SystemExit("hard session budget reason code drifted")
for invariant in (
    "concurrent_expensive_operations_require_reservation_or_equivalent",
    "provider_fallback_cannot_bypass_budget_or_egress_policy",
    "retry_cannot_double_reserve_or_double_charge",
):
    if budget.get(invariant) is not True:
        raise SystemExit(f"hard session budget invariant lost: {invariant}")

expected_actions = {
    "WARN",
    "DEGRADE_QUALITY",
    "FALL_BACK_TO_VOICE",
    "FALL_BACK_TO_TEXT",
    "END_SESSION",
    "REQUIRE_EXPLICIT_APPROVAL",
}
if set(contract.get("allowed_exhaustion_actions", [])) != expected_actions:
    raise SystemExit("budget exhaustion action vocabulary drifted")

expected_fail_closed = {
    ("PROVIDER_CHARGE_MISSING", "REPORT_UNKNOWN_NOT_SYNTHESIZED"),
    ("HARD_SESSION_BUDGET_EXHAUSTED", "DENY_OR_APPLY_PREAPPROVED_EXHAUSTION_POLICY"),
    ("CONCURRENT_RESERVATIONS_EXCEED_REMAINING_BUDGET", "AT_MOST_BUDGETED_OPERATIONS_PROCEED"),
    ("RETRY_AFTER_UNKNOWN_EXTERNAL_OUTCOME", "RECONCILE_BEFORE_NEW_RESERVATION_OR_CHARGE"),
    ("FALLBACK_PROVIDER_COSTS_MORE_OR_VIOLATES_POLICY", "DO_NOT_SILENTLY_FALL_BACK"),
}
actual_fail_closed = {
    (item.get("case"), item.get("decision"))
    for item in contract.get("fail_closed_examples", [])
}
if actual_fail_closed != expected_fail_closed:
    raise SystemExit("RT1 cost/budget fail-closed matrix drifted")

# Pre-entry guard: no production RT1 cost-estimate or hard-budget surface may
# appear in the canonical runtime or user-facing Owner Lab before RT1 entry.
surface_suffixes = {".rs", ".ts", ".tsx", ".js", ".html"}
forbidden_markers = (
    "PrePublicationCostEstimate",
    "PrepublicationCostEstimate",
    "HardSessionBudget",
    "hard_session_budget",
    "prepublication-cost-estimate",
    "/api/cost/estimate",
    "/api/budget/session",
    'id="cost-estimate-before-publication"',
    'id="hard-session-budget"',
)
for source in CRATES.rglob("*"):
    if not source.is_file() or source.suffix not in surface_suffixes:
        continue
    text = source.read_text(encoding="utf-8")
    marker = next((item for item in forbidden_markers if item in text), None)
    if marker is not None:
        raise SystemExit(
            "production RT1 cost/budget surface appeared while RT1 remains blocked "
            f"on RT0 exit: {source.relative_to(ROOT)} (marker={marker!r})"
        )

for required_spec_marker in (
    "Status:** `BLOCKED_ON_RT0_EXIT`",
    "Allowed pre-entry work:** `BOUNDED_FEASIBILITY_SPIKE_ONLY`",
    "first cost estimate before publication/use",
    "Cost and hard-budget feasibility boundary",
    "ESTIMATE != USAGE != PROVIDER CHARGE != CUSTOMER CHARGE",
    "must not silently exceed a hard session budget",
    "must remain contract-only and non-promoting",
):
    if required_spec_marker not in spec:
        raise SystemExit(f"RT1 cost/budget spike lost ReleaseSpec guard: {required_spec_marker}")

print("rt1-cost-budget-spike: PASS")
