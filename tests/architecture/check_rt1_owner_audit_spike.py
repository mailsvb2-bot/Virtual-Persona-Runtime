from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "rt1-owner-audit-0.1.json"
RT1_SPEC = ROOT / "docs" / "releases" / "RT1_RELEASE_SPEC.md"
CRATES = ROOT / "crates"

contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
spec = RT1_SPEC.read_text(encoding="utf-8")

if contract.get("schema") != "rt1-owner-audit-spike-0.1":
    raise SystemExit("RT1 owner audit spike schema drifted")
if contract.get("maturity") != "RESEARCH_REQUIRED":
    raise SystemExit("RT1 owner audit spike must remain RESEARCH_REQUIRED")
for flag in (
    "production",
    "durable_audit_log_implemented",
    "owner_audit_ui_implemented",
    "audit_export_implemented",
):
    if contract.get(flag) is not False:
        raise SystemExit(f"RT1 owner audit spike must remain non-promoting: {flag}")

scope = contract.get("canonical_scope", {})
for invariant in (
    "is_projection_of_authoritative_mutations_and_execution_evidence",
    "is_not_second_persona_or_claim_source_of_truth",
    "owner_visible_where_disclosure_allows",
    "operator_or_admin_visibility_requires_separate_authority",
):
    if scope.get(invariant) is not True:
        raise SystemExit(f"RT1 owner audit scope invariant lost: {invariant}")

expected_families = {
    "PERSONA_CREATED_OR_VERSION_ADVANCED",
    "OWNER_CLAIM_CONFIRMED",
    "OWNER_CLAIM_CORRECTED",
    "OWNER_CLAIM_HIDDEN",
    "OWNER_CLAIM_DELETED",
    "AUDIENCE_RESTRICTED",
    "INFERENCE_REJECTED",
    "CONSENT_OR_RIGHTS_CHANGED",
    "VOICE_OR_APPEARANCE_REPRESENTATION_ACTIVATED",
    "PUBLICATION_CREATED",
    "PUBLICATION_PAUSED",
    "PUBLICATION_RESUMED",
    "PUBLICATION_UNPUBLISHED",
    "PREPARATION_RETRY_OR_CANCEL_REQUESTED",
    "BEHAVIOR_CHANGE_APPROVED",
    "OWNER_VISIBLE_SECURITY_OR_AUTHORITY_DENIAL",
}
if set(contract.get("minimum_audited_event_families", [])) != expected_families:
    raise SystemExit("RT1 owner audit event families drifted")

required_record = {
    "AUDIT_EVENT_ID",
    "EVENT_TYPE",
    "TIMESTAMP",
    "ACTOR_ID_OR_ACTOR_CLASS",
    "WORKSPACE_OR_TENANT_SCOPE",
    "PERSONA_ID",
    "PERSONA_VERSION_WHEN_APPLICABLE",
    "TARGET_ID_WHEN_APPLICABLE",
    "REQUEST_OR_OPERATION_ID_WHEN_APPLICABLE",
    "REASON_CODE_WHEN_APPLICABLE",
    "RESULT",
    "EVIDENCE_OR_REVISION_REFERENCE_WHEN_APPLICABLE",
}
if set(contract.get("record_minimum", [])) != required_record:
    raise SystemExit("RT1 owner audit record minimum drifted")

for section, required in {
    "privacy_and_integrity": (
        "audit_record_does_not_copy_secrets",
        "audit_record_does_not_copy_unrestricted_private_payloads_by_default",
        "deleted_personal_content_need_not_be_retained_in_audit",
        "minimal_non_content_tombstone_allowed_for_deletion_evidence",
        "audit_read_respects_current_disclosure_and_authority",
        "audit_event_identity_is_stable",
        "retry_must_not_create_duplicate_business_audit_event_for_same_committed_effect",
        "append_or_immutable_evidence_semantics_required_for_committed_history",
    ),
    "binding_invariants": (
        "claim_correction_audit_binds_prior_and_new_revision",
        "persona_version_change_audit_binds_old_and_new_version",
        "publication_event_binds_exact_publication_identity_and_persona_version",
        "consent_or_rights_event_binds_authorization_epoch_or_equivalent_revision_when_available",
        "provider_ids_never_replace_canonical_persona_or_representation_identity",
        "answer_evidence_card_remains_separate_turn_evidence_projection",
    ),
    "recovery_and_failure": (
        "durable_mutation_and_required_audit_event_must_not_split_brain",
        "unknown_external_outcome_may_record_reconciling_state_without_claiming_success",
        "failed_mutation_must_not_be_audited_as_committed_success",
        "rollback_or_safe_rollforward_preserves_committed_audit_continuity",
    ),
}.items():
    values = contract.get(section, {})
    for invariant in required:
        if values.get(invariant) is not True:
            raise SystemExit(f"RT1 owner audit invariant lost: {section}.{invariant}")

expected_fail_closed = {
    ("CLAIM_DELETE_SUCCEEDS_BUT_AUDIT_WOULD_REQUIRE_RETAINING_DELETED_CONTENT", "STORE_MINIMAL_NON_CONTENT_DELETION_EVIDENCE_ONLY"),
    ("VISITOR_REQUESTS_OWNER_AUDIT_TRAIL", "DENY"),
    ("RETRIED_MUTATION_ALREADY_COMMITTED", "RETURN_SINGLE_BUSINESS_EFFECT_AND_NON_DUPLICATED_COMMITTED_AUDIT"),
    ("PROVIDER_ACK_LOST", "AUDIT_UNKNOWN_OR_RECONCILING_NOT_SUCCESS"),
    ("AUDIT_READ_WOULD_DISCLOSE_RESTRICTED_PRIVATE_SOURCE", "REDACT_OR_OMIT_BY_EFFECTIVE_AUTHORITY"),
}
actual_fail_closed = {
    (item.get("case"), item.get("decision"))
    for item in contract.get("fail_closed_examples", [])
}
if actual_fail_closed != expected_fail_closed:
    raise SystemExit("RT1 owner audit fail-closed matrix drifted")

surface_suffixes = {".rs", ".ts", ".tsx", ".js", ".html"}
forbidden_markers = (
    "struct Rt1AuditRecord",
    "Rt1OwnerAuditLog",
    "/api/rt1/audit",
    'id="rt1-owner-audit"',
)
for source in CRATES.rglob("*"):
    if not source.is_file() or source.suffix not in surface_suffixes:
        continue
    text = source.read_text(encoding="utf-8")
    marker = next((item for item in forbidden_markers if item in text), None)
    if marker is not None:
        raise SystemExit(
            "production RT1 owner-audit surface appeared while RT1 remains blocked "
            f"on RT0 exit: {source.relative_to(ROOT)} (marker={marker!r})"
        )

for required_spec_marker in (
    "Status:** `BLOCKED_ON_RT0_EXIT`",
    "Allowed pre-entry work:** `BOUNDED_FEASIBILITY_SPIKE_ONLY`",
    "owner-visible audit/provenance minimum needed by this journey",
    "Owner-visible audit feasibility boundary",
    "must not become a second Persona or claim source of truth",
    "must remain contract-only and non-promoting",
):
    if required_spec_marker not in spec:
        raise SystemExit(f"RT1 owner audit spike lost ReleaseSpec guard: {required_spec_marker}")

print("rt1-owner-audit-spike: PASS")
