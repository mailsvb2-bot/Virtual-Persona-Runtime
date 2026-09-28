from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "rt1-reason-ui-0.1.json"
RT1_SPEC = ROOT / "docs" / "releases" / "RT1_RELEASE_SPEC.md"
REASON_SOURCE = ROOT / "crates" / "vpr-domain" / "src" / "reason.rs"
CRATES = ROOT / "crates"

contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
spec = RT1_SPEC.read_text(encoding="utf-8")
reason_source = REASON_SOURCE.read_text(encoding="utf-8")

if contract.get("schema") != "rt1-reason-ui-spike-0.1":
    raise SystemExit("RT1 reason/UI spike schema drifted")
if contract.get("maturity") != "RESEARCH_REQUIRED":
    raise SystemExit("RT1 reason/UI spike must remain RESEARCH_REQUIRED")
for flag in ("production", "rt1_reason_enum_implemented", "production_ui_mapping_implemented"):
    if contract.get(flag) is not False:
        raise SystemExit(f"RT1 reason/UI spike must remain non-promoting: {flag}")
if contract.get("canonical_reason_source") != "vpr_domain::Rt0ReasonCode":
    raise SystemExit("RT1 must reuse canonical Rt0ReasonCode source")

canonical = {
    "AUTH_REVOKED",
    "AUTH_EXPIRED",
    "AUTH_SCOPE_DENIED",
    "EGRESS_DENIED",
    "EGRESS_LOCAL_ONLY",
    "CONSENT_REQUIRED",
    "PROVIDER_UNAVAILABLE",
    "PROVIDER_RATE_LIMITED",
    "PROVIDER_TIMEOUT",
    "PREPARATION_FAILED",
    "TURN_CANCELLED",
    "INVALID_STATE_TRANSITION",
    "OWNER_ATTRIBUTION_UNVERIFIED",
    "BUDGET_EXHAUSTED",
    "INTERNAL_ERROR",
}
if set(contract.get("canonical_reason_codes", [])) != canonical:
    raise SystemExit("RT1 canonical reason code set drifted")

minimum_rt1 = canonical - {"TURN_CANCELLED"}
if set(contract.get("minimum_rt1_journey_codes", [])) != minimum_rt1:
    raise SystemExit("RT1 minimum journey reason-code set drifted")

for code in canonical:
    if f'"{code}"' not in reason_source:
        raise SystemExit(f"canonical reason source lost code: {code}")

ui = contract.get("ui_semantics", {})
if set(ui) != canonical:
    raise SystemExit("every canonical reason code must have a bounded UI semantic")

for code in ("AUTH_REVOKED", "AUTH_EXPIRED", "AUTH_SCOPE_DENIED"):
    if ui.get(code, {}).get("automatic_retry_without_authority_change") is not False:
        raise SystemExit(f"{code} must not auto-retry without authority change")
if ui.get("CONSENT_REQUIRED", {}).get("automatic_retry_without_consent_change") is not False:
    raise SystemExit("CONSENT_REQUIRED must not auto-retry without consent change")
for code in ("EGRESS_DENIED", "EGRESS_LOCAL_ONLY"):
    if ui.get(code, {}).get("silent_external_fallback") is not False:
        raise SystemExit(f"{code} must not silently egress")
for code in ("PROVIDER_UNAVAILABLE", "PROVIDER_RATE_LIMITED", "PROVIDER_TIMEOUT"):
    if ui.get(code, {}).get("silent_fallback") is not False:
        raise SystemExit(f"{code} must not silently switch provider")
if ui.get("PREPARATION_FAILED", {}).get("unrelated_modalities_remain_usable_when_valid") is not True:
    raise SystemExit("preparation failure must preserve unrelated valid modalities")
if ui.get("BUDGET_EXHAUSTED", {}).get("silent_budget_overrun") is not False:
    raise SystemExit("budget exhausted must not overrun silently")
if ui.get("INTERNAL_ERROR", {}).get("raw_secret_or_stack_disclosure") is not False:
    raise SystemExit("internal error must not disclose raw secrets/stacks")

for invariant in (
    "rt1_reuses_canonical_reason_enum",
    "new_rt1_specific_reason_requires_versioned_contract_change",
    "localized_copy_is_not_reason_identity",
    "unknown_reason_never_maps_to_success",
    "ui_action_does_not_bypass_server_authorization",
    "provider_failure_never_maps_to_business_success",
    "expected_denial_is_user_comprehensible_before_production",
    "raw_provider_error_or_secret_not_shown_by_default",
):
    if contract.get("cross_cutting_invariants", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 reason/UI invariant lost: {invariant}")

expected_fail_closed = {
    ("UNKNOWN_REASON_CODE_REACHES_UI", "SHOW_SAFE_GENERIC_FAILURE_NOT_SUCCESS"),
    ("CONSENT_REQUIRED_AND_USER_PRESSES_RETRY_WITHOUT_NEW_CONSENT", "REMAIN_DENIED"),
    ("PROVIDER_TIMEOUT_WITH_UNAPPROVED_FALLBACK", "DO_NOT_SILENTLY_SWITCH_PROVIDER"),
    ("BUDGET_EXHAUSTED_WITH_RETRY", "DO_NOT_EXCEED_HARD_BUDGET"),
    ("AUTH_SCOPE_DENIED_BUT_UI_BUTTON_VISIBLE", "SERVER_DENY_REMAINS_AUTHORITATIVE"),
}
actual_fail_closed = {
    (item.get("case"), item.get("decision"))
    for item in contract.get("fail_closed_examples", [])
}
if actual_fail_closed != expected_fail_closed:
    raise SystemExit("RT1 reason/UI fail-closed matrix drifted")

surface_suffixes = {".rs", ".ts", ".tsx", ".js", ".html"}
forbidden_markers = (
    "enum Rt1ReasonCode",
    "struct Rt1ReasonUiMap",
    "/api/rt1/reasons",
    'id="rt1-reason-catalog"',
)
for source in CRATES.rglob("*"):
    if not source.is_file() or source.suffix not in surface_suffixes:
        continue
    text = source.read_text(encoding="utf-8")
    marker = next((item for item in forbidden_markers if item in text), None)
    if marker is not None:
        raise SystemExit(
            "production RT1 reason/UI surface appeared while RT1 remains blocked "
            f"on RT0 exit: {source.relative_to(ROOT)} (marker={marker!r})"
        )

for required_spec_marker in (
    "Status:** `BLOCKED_ON_RT0_EXIT`",
    "Allowed pre-entry work:** `BOUNDED_FEASIBILITY_SPIKE_ONLY`",
    "Stable reason-code / actionable UI feasibility boundary",
    "reuse the existing canonical `Rt0ReasonCode`",
    "unknown reason must never render as success",
    "must remain contract-only and non-promoting",
):
    if required_spec_marker not in spec:
        raise SystemExit(f"RT1 reason/UI spike lost ReleaseSpec guard: {required_spec_marker}")

print("rt1-reason-ui-spike: PASS")
