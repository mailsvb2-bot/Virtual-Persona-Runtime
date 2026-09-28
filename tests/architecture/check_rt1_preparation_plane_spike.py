from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "rt1-preparation-plane-0.1.json"
RT1_SPEC = ROOT / "docs" / "releases" / "RT1_RELEASE_SPEC.md"
CRATES = ROOT / "crates"

contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
spec = RT1_SPEC.read_text(encoding="utf-8")

if contract.get("schema") != "rt1-preparation-plane-spike-0.1":
    raise SystemExit("RT1 preparation-plane spike schema drifted")
if contract.get("maturity") != "RESEARCH_REQUIRED":
    raise SystemExit("RT1 preparation-plane spike must remain RESEARCH_REQUIRED")
for flag in (
    "production",
    "durable_preparation_jobs_implemented",
    "owner_preparation_ui_implemented",
    "production_retry_cancel_implemented",
):
    if contract.get(flag) is not False:
        raise SystemExit(f"RT1 preparation-plane spike must remain non-promoting: {flag}")

expected_states = {
    "QUEUED",
    "PREPARING",
    "VALIDATING",
    "READY",
    "FAILED",
    "CANCELLED",
    "EXPIRED",
}
if set(contract.get("job_states", [])) != expected_states:
    raise SystemExit("RT1 preparation job state vocabulary drifted")

for invariant in (
    "preparation_plane_separate_from_live_plane",
    "prepared_assets_reusable",
    "preparation_failure_does_not_destroy_persona",
    "partial_readiness_is_not_total_persona_failure",
):
    if contract.get("planes", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 preparation plane invariant lost: {invariant}")

for invariant in (
    "stable_job_id",
    "binds_persona_id",
    "binds_persona_version_or_input_revision",
    "binds_modality_or_asset_kind",
    "binds_provider_operation_when_external",
    "provider_id_not_canonical_persona_identity",
):
    if contract.get("job_identity_and_binding", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 preparation job binding invariant lost: {invariant}")

for invariant in (
    "retry_targets_only_affected_job_or_representation",
    "retry_is_idempotent_when_repeated",
    "unknown_external_outcome_reconciled_before_retry",
    "local_cancel_does_not_imply_provider_cancel_or_refund",
    "late_provider_success_after_cancel_cannot_silently_publish",
):
    if contract.get("retry_and_cancel", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 preparation retry/cancel invariant lost: {invariant}")

for invariant in (
    "result_for_old_persona_version_or_input_revision_not_current",
    "material_provider_or_config_change_requires_revalidation",
    "correction_invalidates_only_affected_derived_or_prepared_state",
    "unrelated_ready_modalities_remain_intact",
):
    if contract.get("stale_result_rules", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 stale preparation-result invariant lost: {invariant}")

expected_fail_closed = {
    ("VOICE_PREPARATION_FAILED", "VOICE_FAILED_PERSONA_AND_UNRELATED_MODALITIES_INTACT"),
    ("VIDEO_RETRY_REQUESTED", "RETRY_ONLY_VIDEO_PREPARATION"),
    ("OLD_JOB_COMPLETES_AFTER_PERSONA_CORRECTION", "MARK_STALE_DO_NOT_PROMOTE_CURRENT_READINESS"),
    ("CANCELLED_EXTERNAL_JOB_LATER_REPORTS_SUCCESS", "RECONCILE_AND_REQUIRE_CURRENT_BINDING_BEFORE_ACCEPTANCE"),
    ("PROVIDER_ACK_LOST", "UNKNOWN_OUTCOME_RECONCILE_BEFORE_RETRY"),
}
actual_fail_closed = {
    (item.get("case"), item.get("decision"))
    for item in contract.get("fail_closed_examples", [])
}
if actual_fail_closed != expected_fail_closed:
    raise SystemExit("RT1 preparation-plane fail-closed matrix drifted")

# RT0 already has experimental in-memory modality preparation/readiness.
# This guard blocks a distinct durable RT1 product surface from sneaking in
# before RT0 exit without falsely banning the existing RT0 proof code.
surface_suffixes = {".rs", ".ts", ".tsx", ".js", ".html"}
forbidden_markers = (
    "DurablePreparationJob",
    "PreparationJobRecord",
    "Rt1PreparationJob",
    "/api/preparation/jobs",
    "/api/preparation/job/",
    'id="preparation-jobs"',
    'id="preparation-job-history"',
)
for source in CRATES.rglob("*"):
    if not source.is_file() or source.suffix not in surface_suffixes:
        continue
    text = source.read_text(encoding="utf-8")
    marker = next((item for item in forbidden_markers if item in text), None)
    if marker is not None:
        raise SystemExit(
            "production RT1 preparation-job surface appeared while RT1 remains blocked "
            f"on RT0 exit: {source.relative_to(ROOT)} (marker={marker!r})"
        )

for required_spec_marker in (
    "Status:** `BLOCKED_ON_RT0_EXIT`",
    "Allowed pre-entry work:** `BOUNDED_FEASIBILITY_SPIKE_ONLY`",
    "Preparation Plane job lifecycle",
    "Preparation Plane job feasibility boundary",
    "QUEUED -> PREPARING -> VALIDATING -> READY | FAILED | CANCELLED | EXPIRED",
    "Retry must target only the affected representation",
    "must remain contract-only and non-promoting",
):
    if required_spec_marker not in spec:
        raise SystemExit(f"RT1 preparation-plane spike lost ReleaseSpec guard: {required_spec_marker}")

print("rt1-preparation-plane-spike: PASS")
