from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "rt1-api-boundary-0.1.json"
RT1_SPEC = ROOT / "docs" / "releases" / "RT1_RELEASE_SPEC.md"
CRATES = ROOT / "crates"

contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
spec = RT1_SPEC.read_text(encoding="utf-8")

if contract.get("schema") != "rt1-api-boundary-spike-0.1":
    raise SystemExit("RT1 API boundary spike schema drifted")
if contract.get("maturity") != "RESEARCH_REQUIRED":
    raise SystemExit("RT1 API boundary spike must remain RESEARCH_REQUIRED")
for flag in (
    "production",
    "rt1_http_endpoints_implemented",
    "public_api_implemented",
    "durable_mutation_api_implemented",
):
    if contract.get(flag) is not False:
        raise SystemExit(f"RT1 API boundary spike must remain non-promoting: {flag}")

expected_families = {
    "OWNER_WORKSPACE_SESSION_IDENTITY",
    "PERSONA_CREATE_READ",
    "CAPTURE_IMPORT_INITIATE_COMPLETE",
    "OWNER_CLAIM_LIST_REVIEW_CORRECT_HIDE_DELETE",
    "PREPARATION_CREATE_LIST_RETRY_CANCEL",
    "READINESS_QUERY",
    "VISITOR_PREVIEW_SESSION",
    "PUBLICATION_CREATE_READ_PAUSE_RESUME_UNPUBLISH",
    "PUBLIC_VISITOR_SESSION_BOOTSTRAP",
    "ANSWER_EVIDENCE_CARD_RETRIEVAL",
    "PRE_PUBLICATION_OR_USE_COST_ESTIMATE",
    "OWNER_VISIBLE_AUDIT_PROVENANCE",
}
if set(contract.get("operation_families", [])) != expected_families:
    raise SystemExit("RT1 API operation families drifted")

for section, required in {
    "versioning": (
        "api_contract_is_versioned",
        "breaking_change_requires_explicit_version_or_migration_path",
        "provider_specific_ids_not_required_in_normal_owner_payloads",
        "saved_publication_links_must_not_silently_change_semantics",
    ),
    "mutation_semantics": (
        "mutation_requires_stable_request_identity",
        "retry_with_same_request_identity_must_not_duplicate_business_effect",
        "mutable_aggregate_write_requires_expected_version_or_equivalent_concurrency_token",
        "stale_write_fails_closed",
        "owner_mutation_requires_current_owner_authority",
        "workspace_or_tenant_scope_checked_server_side",
        "durable_state_and_required_event_must_not_split_brain",
    ),
    "external_side_effect_semantics": (
        "idempotency_used_when_provider_supports_it",
        "external_operation_id_preserved_when_available",
        "unknown_outcome_reconciled_before_unsafe_retry",
        "local_cancel_does_not_imply_provider_cancel_or_refund",
    ),
    "pagination": (
        "list_operations_use_bounded_page_size",
        "cursor_is_opaque_to_clients",
        "ordering_is_explicit_and_stable_within_contract",
        "cursor_must_not_encode_owner_secret_or_provider_credential",
        "pagination_must_not_bypass_authorization_or_disclosure_filters",
    ),
    "read_and_projection_semantics": (
        "persona_reads_bind_canonical_persona_identity_and_version",
        "public_bootstrap_requires_exact_current_publication_binding",
        "visitor_preview_executes_with_visitor_permissions",
        "evidence_reads_respect_effective_disclosure",
        "stale_or_ambiguous_public_state_fails_closed",
    ),
    "error_contract": (
        "stable_reason_code_required_for_expected_domain_denial_or_failure",
        "unknown_internal_failure_maps_to_internal_error_without_secret_leak",
        "authorization_denial_must_not_be_returned_as_success",
        "provider_failure_must_not_be_returned_as_business_success",
        "user_actionable_ui_mapping_required_before_production",
    ),
    "security_and_input": (
        "server_side_validation_required",
        "unexpected_enum_rejected",
        "oversized_payload_bounded",
        "duplicate_or_ambiguous_fields_rejected_or_canonicalized_by_explicit_contract",
        "secrets_and_raw_private_source_payloads_not_echoed_by_default",
        "visitor_cannot_call_owner_mutation_surface",
    ),
}.items():
    values = contract.get(section, {})
    for invariant in required:
        if values.get(invariant) is not True:
            raise SystemExit(f"RT1 API invariant lost: {section}.{invariant}")

expected_fail_closed = {
    ("RETRIED_MUTATION_WITH_SAME_REQUEST_ID", "RETURN_OR_RECONCILE_SINGLE_BUSINESS_EFFECT"),
    ("MUTATION_WITH_STALE_EXPECTED_PERSONA_VERSION", "DENY_STALE_WRITE"),
    ("UNKNOWN_PROVIDER_OUTCOME_THEN_RETRY", "RECONCILE_BEFORE_NEW_SIDE_EFFECT"),
    ("VISITOR_ATTEMPTS_OWNER_MUTATION", "DENY_BEFORE_PROVIDER_EGRESS"),
    ("PAGINATION_CURSOR_FROM_DIFFERENT_AUTHORITY_SCOPE", "DENY_OR_RESTART_AUTHORIZED_LIST"),
}
actual_fail_closed = {
    (item.get("case"), item.get("decision"))
    for item in contract.get("fail_closed_examples", [])
}
if actual_fail_closed != expected_fail_closed:
    raise SystemExit("RT1 API fail-closed matrix drifted")

surface_suffixes = {".rs", ".ts", ".tsx", ".js", ".html"}
forbidden_markers = (
    "/api/rt1/",
    "Rt1ApiRouter",
    "Rt1PublicApi",
    'id="rt1-public-api"',
)
for source in CRATES.rglob("*"):
    if not source.is_file() or source.suffix not in surface_suffixes:
        continue
    text = source.read_text(encoding="utf-8")
    marker = next((item for item in forbidden_markers if item in text), None)
    if marker is not None:
        raise SystemExit(
            "production RT1 API surface appeared while RT1 remains blocked "
            f"on RT0 exit: {source.relative_to(ROOT)} (marker={marker!r})"
        )

for required_spec_marker in (
    "Status:** `BLOCKED_ON_RT0_EXIT`",
    "Allowed pre-entry work:** `BOUNDED_FEASIBILITY_SPIKE_ONLY`",
    "Exact transport payloads, idempotency keys, pagination and concurrency controls must be frozen",
    "API contract feasibility boundary",
    "stable request identity",
    "opaque cursor",
    "must remain contract-only and non-promoting",
):
    if required_spec_marker not in spec:
        raise SystemExit(f"RT1 API boundary spike lost ReleaseSpec guard: {required_spec_marker}")

print("rt1-api-boundary-spike: PASS")
