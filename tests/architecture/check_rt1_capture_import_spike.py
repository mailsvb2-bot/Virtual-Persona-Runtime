from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "rt1-capture-import-0.1.json"
RT1_SPEC = ROOT / "docs" / "releases" / "RT1_RELEASE_SPEC.md"
CRATES = ROOT / "crates"

contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
spec = RT1_SPEC.read_text(encoding="utf-8")

if contract.get("schema") != "rt1-capture-import-spike-0.1":
    raise SystemExit("RT1 capture/import spike schema drifted")
if contract.get("maturity") != "RESEARCH_REQUIRED":
    raise SystemExit("RT1 capture/import spike must remain RESEARCH_REQUIRED")
for flag in (
    "production",
    "capture_import_runtime_implemented",
    "durable_capture_state_implemented",
    "owner_capture_import_ui_implemented",
):
    if contract.get(flag) is not False:
        raise SystemExit(f"RT1 capture/import spike must remain non-promoting: {flag}")

expected_inputs = {
    "STRUCTURED_OWNER_INTERVIEW",
    "VOICE_SAMPLE",
    "VIDEO_SAMPLE",
    "OWNER_PROVIDED_DOCUMENT_OR_SOURCE",
    "PREFERRED_ANSWER_EXAMPLE",
    "EXPLICIT_BOUNDARY_OR_PROHIBITED_ATTRIBUTION",
}
if set(contract.get("supported_input_kinds", [])) != expected_inputs:
    raise SystemExit("RT1 capture/import input vocabulary drifted")

expected_records = {
    "IDENTITY",
    "COMMUNICATION_STYLE",
    "BIOGRAPHY",
    "PREFERENCES",
    "OWNER_OPINIONS",
    "EXPERTISE",
    "PRONUNCIATION",
    "BEHAVIOR",
    "CONSTITUTION_CLAUSE",
}
if set(contract.get("candidate_record_kinds", [])) != expected_records:
    raise SystemExit("RT1 capture/import candidate record vocabulary drifted")

for invariant in (
    "capture_produces_candidates_not_automatic_truth",
    "automatically_inferred_owner_facts_remain_unverified_until_review_or_approved_verification",
    "automatically_inferred_owner_opinions_remain_unverified_until_review_or_approved_verification",
    "capture_completion_distinct_from_owner_review_completion",
    "candidate_correction_routes_to_canonical_claim_or_behavior_authority",
    "preferred_answer_does_not_silently_become_verified_fact_or_opinion",
):
    if contract.get("candidate_invariants", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 capture candidate invariant lost: {invariant}")

for invariant in (
    "voice_samples_route_to_voice_identity_and_preparation",
    "video_or_appearance_samples_route_to_appearance_identity_and_preparation",
    "real_person_references_require_current_consent_rights_gate",
    "reference_validation_required_before_activation",
    "quality_evaluation_required_before_activation",
    "provider_ids_are_not_owner_facing_identity",
    "reference_assets_do_not_become_persona_claim_truth",
):
    if contract.get("voice_and_appearance_reference_routing", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 media-reference routing invariant lost: {invariant}")

for invariant in (
    "capture_quality_separate_from_embodiment_quality",
    "failed_voice_preparation_does_not_invalidate_reviewed_owner_knowledge",
    "failed_voice_preparation_does_not_invalidate_persona_identity",
    "failed_avatar_preparation_does_not_invalidate_reviewed_owner_knowledge",
    "failed_avatar_preparation_does_not_invalidate_persona_identity",
    "text_capture_can_remain_valid_when_media_preparation_fails",
):
    if contract.get("capture_separation", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 capture separation invariant lost: {invariant}")

for invariant in (
    "import_records_source_or_provenance_reference",
    "import_does_not_imply_consent_or_ownership",
    "raw_voice_video_retention_is_purpose_bound",
    "raw_voice_video_not_retained_indefinitely_by_default",
    "sensitive_reference_access_requires_stricter_acl",
    "production_test_fixtures_should_prefer_synthetic_or_deidentified_data",
):
    if contract.get("source_and_retention", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 capture source/retention invariant lost: {invariant}")

for invariant in (
    "same_request_identity_must_not_duplicate_candidate_records",
    "capture_finish_retry_must_not_duplicate_reviewable_state",
    "provider_retry_must_not_duplicate_external_side_effect_when_idempotency_or_reconciliation_is_required",
    "unknown_external_outcome_requires_reconciliation_before_unsafe_retry",
):
    if contract.get("retry_and_completion", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 capture retry invariant lost: {invariant}")

for invariant in (
    "normal_flow_does_not_require_provider_names",
    "normal_flow_does_not_require_provider_model_ids",
    "normal_flow_does_not_require_vector_store_ids",
    "owner_can_test_candidate_before_publication",
):
    if contract.get("owner_experience", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 capture owner-experience invariant lost: {invariant}")

expected_fail_closed = {
    ("INFERRED_OWNER_OPINION_AUTO_MARKED_OWNER_VERIFIED", "DENY_MISATTRIBUTION"),
    ("REAL_PERSON_VOICE_SAMPLE_WITHOUT_APPLICABLE_CONSENT", "DENY_PREPARATION_OR_ACTIVATION"),
    ("IMPORTED_SOURCE_WITH_AMBIGUOUS_PROVENANCE", "KEEP_CANDIDATE_UNVERIFIED_AND_REQUIRE_REVIEW"),
    ("AVATAR_PREPARATION_FAILS_AFTER_REVIEWED_TEXT_CAPTURE", "PRESERVE_PERSONA_AND_REVIEWED_TEXT_STATE"),
    ("DUPLICATE_CAPTURE_COMPLETION_RETRY", "NO_DUPLICATE_CANDIDATE_OR_REVIEW_EFFECT"),
}
actual_fail_closed = {
    (item.get("case"), item.get("decision"))
    for item in contract.get("fail_closed_examples", [])
}
if actual_fail_closed != expected_fail_closed:
    raise SystemExit("RT1 capture/import fail-closed matrix drifted")

surface_suffixes = {".rs", ".ts", ".tsx", ".js", ".html"}
forbidden_markers = (
    "Rt1CaptureImport",
    "DurableCaptureImportRecord",
    "/api/rt1/capture/import",
    'id="rt1-capture-import"',
)
for source in CRATES.rglob("*"):
    if not source.is_file() or source.suffix not in surface_suffixes:
        continue
    text = source.read_text(encoding="utf-8")
    marker = next((item for item in forbidden_markers if item in text), None)
    if marker is not None:
        raise SystemExit(
            "production RT1 capture/import surface appeared while RT1 remains blocked "
            f"on RT0 exit: {source.relative_to(ROOT)} (marker={marker!r})"
        )

for required_spec_marker in (
    "Status:** `BLOCKED_ON_RT0_EXIT`",
    "Allowed pre-entry work:** `BOUNDED_FEASIBILITY_SPIKE_ONLY`",
    "owner capture/import of voice and appearance references",
    "Capture / import feasibility boundary",
    "candidate structured records",
    "Capture quality remains separate from embodiment quality",
    "must remain contract-only and non-promoting",
):
    if required_spec_marker not in spec:
        raise SystemExit(f"RT1 capture/import spike lost ReleaseSpec guard: {required_spec_marker}")

print("rt1-capture-import-spike: PASS")
