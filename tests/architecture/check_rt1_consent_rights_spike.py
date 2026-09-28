from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "rt1-consent-rights-0.1.json"
RT1_SPEC = ROOT / "docs" / "releases" / "RT1_RELEASE_SPEC.md"
CRATES = ROOT / "crates"

contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
spec = RT1_SPEC.read_text(encoding="utf-8")

if contract.get("schema") != "rt1-consent-rights-spike-0.1":
    raise SystemExit("RT1 consent/rights spike schema drifted")
if contract.get("maturity") != "RESEARCH_REQUIRED":
    raise SystemExit("RT1 consent/rights spike must remain RESEARCH_REQUIRED")
for flag in (
    "production",
    "durable_consent_rights_state_implemented",
    "owner_consent_ui_implemented",
    "revocation_runtime_implemented",
):
    if contract.get(flag) is not False:
        raise SystemExit(f"RT1 consent/rights spike must remain non-promoting: {flag}")

expected_dimensions = {
    "RIGHTS_BASIS",
    "CONSENT_STATE",
    "COMMERCIAL_PERMISSION",
    "USAGE_RESTRICTIONS",
    "TERRITORY",
    "EXPIRY",
    "REVOCATION_STATE",
}
if set(contract.get("dimensions", [])) != expected_dimensions:
    raise SystemExit("RT1 consent/rights dimensions drifted")

for invariant in (
    "dimensions_not_collapsed_into_single_enum",
    "persona_version_immutable_authorization_mutable",
    "provider_representation_does_not_own_consent",
    "provider_success_cannot_override_rights",
):
    if contract.get("independence_invariants", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 consent/rights invariant lost: {invariant}")

for invariant in (
    "voice_requires_explicit_consent_scope",
    "face_requires_explicit_consent_scope",
    "body_requires_explicit_consent_scope",
    "scope_binds_allowed_use",
    "scope_binds_audience_or_application_when_relevant",
    "scope_binds_expiry_when_present",
    "commercial_use_requires_separate_permission",
    "raw_biometric_material_not_retained_indefinitely_by_default",
):
    if contract.get("real_person_biometric_rules", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 biometric consent invariant lost: {invariant}")

for invariant in (
    "current_consent_required",
    "current_rights_basis_required",
    "commercial_permission_required_for_commercial_use",
    "usage_restrictions_must_allow_operation",
    "territory_must_allow_operation_when_restricted",
    "expiry_must_not_be_passed",
    "revoked_state_denies_activation_or_use",
):
    if contract.get("activation_gates", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 consent activation gate lost: {invariant}")

for invariant in (
    "revocation_does_not_require_new_persona_version",
    "revocation_invalidates_affected_current_authorization",
    "revoked_representation_not_reactivated_by_stale_cache",
    "revocation_blocks_new_sensitive_operations_before_provider_egress",
    "running_session_propagation_sla_deferred_to_rt2",
    "offline_revocation_semantics_deferred_to_rt2",
):
    if contract.get("revocation_and_change", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 revocation invariant lost: {invariant}")

for invariant in (
    "capture_or_import_records_source_and_scope",
    "import_does_not_imply_ownership_or_consent",
    "third_party_or_ambiguous_rights_fail_closed",
    "reference_validation_distinct_from_consent_validation",
    "quality_validation_distinct_from_rights_validation",
):
    if contract.get("capture_import", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 capture/import rights invariant lost: {invariant}")

expected_fail_closed = {
    ("REAL_PERSON_VOICE_WITHOUT_EXPLICIT_CONSENT_SCOPE", "DENY_BEFORE_PREPARATION_OR_USE"),
    ("FACE_REFERENCE_IMPORTED_BUT_RIGHTS_AMBIGUOUS", "DENY_ACTIVATION"),
    ("COMMERCIAL_USE_WITH_NONCOMMERCIAL_PERMISSION", "DENY"),
    ("EXPIRED_CONSENT", "DENY"),
    ("REVOKED_REPRESENTATION_CACHED_AS_READY", "DENY_AND_INVALIDATE_AFFECTED_AUTHORIZATION"),
}
actual_fail_closed = {
    (item.get("case"), item.get("decision"))
    for item in contract.get("fail_closed_examples", [])
}
if actual_fail_closed != expected_fail_closed:
    raise SystemExit("RT1 consent/rights fail-closed matrix drifted")

surface_suffixes = {".rs", ".ts", ".tsx", ".js", ".html"}
forbidden_markers = (
    "struct RightsBasis",
    "Rt1ConsentRightsRecord",
    "DurableConsentRightsState",
    "struct CommercialPermission",
    "struct UsageRestrictions",
    "struct RevocationState",
    "/api/consent",
    "/api/rights",
    'id="consent-rights-manager"',
)
for source in CRATES.rglob("*"):
    if not source.is_file() or source.suffix not in surface_suffixes:
        continue
    text = source.read_text(encoding="utf-8")
    marker = next((item for item in forbidden_markers if item in text), None)
    if marker is not None:
        raise SystemExit(
            "production RT1 consent/rights surface appeared while RT1 remains blocked "
            f"on RT0 exit: {source.relative_to(ROOT)} (marker={marker!r})"
        )

for required_spec_marker in (
    "Status:** `BLOCKED_ON_RT0_EXIT`",
    "Allowed pre-entry work:** `BOUNDED_FEASIBILITY_SPIKE_ONLY`",
    "consent/rights needed for the supported flow",
    "Consent / rights feasibility boundary",
    "Real-person voice/face/body references require explicit consent scope",
    "must remain contract-only and non-promoting",
):
    if required_spec_marker not in spec:
        raise SystemExit(f"RT1 consent/rights spike lost ReleaseSpec guard: {required_spec_marker}")

print("rt1-consent-rights-spike: PASS")
