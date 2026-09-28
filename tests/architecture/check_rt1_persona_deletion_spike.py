from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "rt1-persona-deletion-0.1.json"
RT1_SPEC = ROOT / "docs" / "releases" / "RT1_RELEASE_SPEC.md"
CRATES = ROOT / "crates"

c = json.loads(CONTRACT.read_text(encoding="utf-8"))
spec = RT1_SPEC.read_text(encoding="utf-8")

if c.get("schema") != "rt1-persona-deletion-spike-0.1":
    raise SystemExit("RT1 Persona deletion schema drifted")
if c.get("maturity") != "RESEARCH_REQUIRED":
    raise SystemExit("RT1 Persona deletion spike must remain RESEARCH_REQUIRED")
for flag in (
    "production",
    "persona_deletion_runtime_implemented",
    "durable_erasure_ledger_implemented",
    "owner_delete_ui_implemented",
):
    if c.get(flag) is not False:
        raise SystemExit(f"RT1 Persona deletion spike must remain non-promoting: {flag}")

for key in (
    "unpublish_is_not_private_persona_deletion",
    "pause_is_not_private_persona_deletion",
    "claim_delete_is_not_private_persona_deletion",
    "private_persona_delete_requires_explicit_owner_authority",
):
    if c.get("semantic_distinctions", {}).get(key) is not True:
        raise SystemExit(f"Persona deletion semantic distinction lost: {key}")

for key in (
    "blocks_new_owner_and_public_use_of_deleted_persona",
    "removes_active_persona_content_and_active_private_state",
    "invalidates_publication_and_preview_bindings",
    "invalidates_or_erases_derived_summaries",
    "invalidates_or_erases_caches",
    "invalidates_or_erases_search_or_vector_index_entries",
    "invalidates_or_erases_generated_profiles_or_equivalent_derived_state",
    "revokes_or_deactivates_affected_voice_and_appearance_representations",
    "does_not_delete_unrelated_workspace_or_other_personas",
):
    if c.get("delete_effects", {}).get(key) is not True:
        raise SystemExit(f"Persona deletion effect invariant lost: {key}")

for key in (
    "deleted_personal_content_not_retained_for_revision_history",
    "minimal_non_content_erasure_tombstone_allowed",
    "tombstone_must_not_contain_deleted_content",
    "deletion_event_auditable_without_preserving_deleted_payload",
    "deletion_verification_required",
):
    if c.get("erasure_and_audit", {}).get(key) is not True:
        raise SystemExit(f"Persona deletion erasure/audit invariant lost: {key}")

for key in (
    "restore_must_not_silently_resurrect_deleted_personal_content",
    "rollback_must_preserve_erasure_intent",
    "ambiguous_restore_state_fails_closed_deleted_or_unavailable",
    "retry_same_delete_request_must_not_duplicate_side_effects",
):
    if c.get("recovery_and_backup", {}).get(key) is not True:
        raise SystemExit(f"Persona deletion recovery invariant lost: {key}")

for key in (
    "delete_requires_exact_current_persona_identity",
    "stale_persona_version_delete_fails_closed",
    "concurrent_correction_or_publication_cannot_overwrite_committed_delete",
    "derived_or_public_state_bound_to_deleted_persona_must_not_become_current_again",
):
    if c.get("concurrency_and_binding", {}).get(key) is not True:
        raise SystemExit(f"Persona deletion binding invariant lost: {key}")

expected = {
    ("UNPUBLISH_REQUEST", "KEEP_PRIVATE_PERSONA"),
    ("DELETE_PERSONA_WITH_STALE_VERSION", "DENY"),
    ("BACKUP_RESTORE_CONTAINS_DELETED_PERSONA_CONTENT", "DO_NOT_RESURRECT_DELETED_CONTENT"),
    ("DELETE_RETRY_AFTER_COMMITTED_DELETE", "IDEMPOTENT_ALREADY_DELETED"),
    ("PUBLICATION_CACHE_REFERENCES_DELETED_PERSONA", "DENY_AND_INVALIDATE_STALE_PROJECTION"),
}
actual = {(x.get("case"), x.get("decision")) for x in c.get("fail_closed_examples", [])}
if actual != expected:
    raise SystemExit("Persona deletion fail-closed matrix drifted")

forbidden = (
    "struct Rt1PersonaDeletion",
    "Rt1PersonaErasureLedger",
    "/api/rt1/persona/delete",
    'id="rt1-persona-delete"',
)
for source in CRATES.rglob("*"):
    if not source.is_file() or source.suffix not in {".rs", ".ts", ".tsx", ".js", ".html"}:
        continue
    text = source.read_text(encoding="utf-8")
    marker = next((m for m in forbidden if m in text), None)
    if marker is not None:
        raise SystemExit(
            "production RT1 Persona deletion surface appeared before RT1 entry: "
            f"{source.relative_to(ROOT)} ({marker})"
        )

for marker in (
    "Status:** `BLOCKED_ON_RT0_EXIT`",
    "Deletion semantics are explicit and must distinguish private Persona deletion from public unpublication.",
    "Private Persona deletion feasibility boundary",
    "restore MUST NOT silently resurrect deleted personal content",
    "must remain contract-only and non-promoting",
):
    if marker not in spec:
        raise SystemExit(f"Persona deletion spike lost ReleaseSpec guard: {marker}")

print("rt1-persona-deletion-spike: PASS")
