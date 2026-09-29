from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "rt1-public-session-lease-0.1.json"
RT1_SPEC = ROOT / "docs" / "releases" / "RT1_RELEASE_SPEC.md"
PUBLICATION = ROOT / "contracts" / "rt1-publication-boundary-0.1.json"

contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
spec = RT1_SPEC.read_text(encoding="utf-8")
publication = json.loads(PUBLICATION.read_text(encoding="utf-8"))

if contract.get("schema") != "rt1-public-session-lease-spike-0.1":
    raise SystemExit("RT1 public-session lease spike schema drifted")
if contract.get("maturity") != "RESEARCH_REQUIRED":
    raise SystemExit("pre-entry public-session lease spike must remain RESEARCH_REQUIRED")
for field in ("production", "public_endpoint_exposed", "durable_state_implemented"):
    if contract.get(field) is not False:
        raise SystemExit(f"RT1 public-session lease spike must remain non-promoting: {field}")

binding = contract.get("lease_binding", {})
for required in (
    "binds_publication_id",
    "binds_persona_id",
    "binds_persona_version",
    "binds_authorization_epoch",
    "binds_publication_state_version",
    "bounded_expiry_required",
):
    if binding.get(required) is not True:
        raise SystemExit(f"public-session lease lost authority binding: {required}")
if binding.get("provider_session_id_is_not_authority") is not True:
    raise SystemExit("provider session identity must never become public authority")
if binding.get("client_token_is_not_canonical_authority") is not True:
    raise SystemExit("client token must not become canonical publication authority")

issuance = contract.get("issuance", {})
for required in (
    "allowed_only_from_published",
    "requires_exact_current_persona_version",
    "requires_current_authorization_epoch",
    "requires_current_publication_state_version",
    "ambiguous_state_denied",
):
    if issuance.get(required) is not True:
        raise SystemExit(f"public-session lease issuance must fail closed: {required}")

revalidation = contract.get("revalidation", {})
for required in (
    "required_before_new_turn_or_protected_provider_egress",
    "pause_invalidates_active_public_authority",
    "unpublish_invalidates_active_public_authority",
    "consent_or_permission_revoke_invalidates_active_public_authority",
    "persona_correction_invalidates_stale_version_lease",
    "expired_lease_denied",
    "cache_cannot_extend_authority",
):
    if revalidation.get(required) is not True:
        raise SystemExit(f"active public-session revalidation drifted: {required}")

concurrency = contract.get("concurrency", {})
for required in (
    "state_and_epoch_changes_are_monotonic",
    "revocation_wins_over_concurrent_resume_for_older_epoch",
    "late_provider_success_cannot_restore_authority",
    "stale_lease_cannot_be_reissued_as_current_without_new_authorization_check",
    "retry_does_not_duplicate_authority_grant",
):
    if concurrency.get(required) is not True:
        raise SystemExit(f"public-session concurrency safety drifted: {required}")

active = contract.get("active_session_after_revocation", {})
if active.get("new_turn_denied") is not True:
    raise SystemExit("revoked active public session must deny the next turn")
if active.get("in_flight_turn_cancelled_or_drained_without_further_protected_egress") is not True:
    raise SystemExit("revoked in-flight turn must stop protected egress")
if active.get("client_disconnect_not_required_for_server_side_denial") is not True:
    raise SystemExit("server-side denial cannot depend on client disconnect")
if active.get("provider_disconnect_best_effort_not_source_of_truth") is not True:
    raise SystemExit("provider disconnect cannot be canonical authority")

expected_cases = {
    "PUBLICATION_PAUSED_WITH_ACTIVE_SESSION": "DENY_NEXT_TURN_AND_REVOKE_PUBLIC_AUTHORITY",
    "PUBLICATION_UNPUBLISHED_WITH_ACTIVE_SESSION": "DENY_NEXT_TURN_AND_REVOKE_PUBLIC_AUTHORITY",
    "PERSONA_CORRECTED_AFTER_LEASE_ISSUED": "DENY_STALE_PERSONA_VERSION_LEASE",
    "CONSENT_REVOKED_DURING_IN_FLIGHT_TURN": "CANCEL_OR_DRAIN_WITHOUT_FURTHER_PROTECTED_EGRESS",
    "STALE_RESUME_RACES_WITH_NEWER_REVOKE": "REVOKE_WINS_OLDER_EPOCH_CANNOT_REAUTHORIZE",
    "PROVIDER_REPORTS_LATE_SUCCESS_AFTER_REVOKE": "IGNORE_FOR_AUTHORITY_RECONCILE_AS_NON_CURRENT",
}
actual_cases = {item.get("case"): item.get("decision") for item in contract.get("fail_closed_examples", [])}
if actual_cases != expected_cases:
    raise SystemExit("public-session lease fail-closed examples are incomplete")

public_bootstrap = publication.get("bootstrap", {}).get("public_visitor", {})
if public_bootstrap.get("allowed_states") != ["PUBLISHED"]:
    raise SystemExit("public-session lease spike no longer composes with publication bootstrap contract")
if public_bootstrap.get("requires_exact_persona_version") is not True:
    raise SystemExit("public-session lease must preserve exact PersonaVersion bootstrap binding")

for marker in (
    "Status:** `BLOCKED_ON_RT0_EXIT`",
    "Allowed pre-entry work:** `BOUNDED_FEASIBILITY_SPIKE_ONLY`",
    "Existing sessions follow explicit lease/revocation semantics",
    "Public session lease / revocation feasibility boundary",
    "must remain contract-only and non-promoting",
):
    if marker not in spec:
        raise SystemExit(f"RT1 public-session lease spike lost ReleaseSpec guard: {marker}")

print("rt1-public-session-lease-spike: PASS")
