from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "rt1-persona-lifecycle-0.1.json"
RT1_SPEC = ROOT / "docs" / "releases" / "RT1_RELEASE_SPEC.md"
CRATES = ROOT / "crates"

contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
spec = RT1_SPEC.read_text(encoding="utf-8")

if contract.get("schema") != "rt1-persona-lifecycle-spike-0.1":
    raise SystemExit("RT1 Persona lifecycle spike schema drifted")
if contract.get("maturity") != "RESEARCH_REQUIRED":
    raise SystemExit("RT1 Persona lifecycle spike must remain RESEARCH_REQUIRED")
for flag in (
    "production",
    "persona_lifecycle_runtime_implemented",
    "owner_creation_ui_implemented",
    "production_publication_transition_implemented",
):
    if contract.get(flag) is not False:
        raise SystemExit(f"RT1 Persona lifecycle spike must remain non-promoting: {flag}")

expected_states = {
    "DRAFT",
    "CAPTURED",
    "REVIEWED",
    "READY_FOR_PREVIEW",
    "READY_FOR_PUBLICATION",
    "PUBLISHED",
    "PAUSED",
    "UNPUBLISHED",
}
if set(contract.get("states", [])) != expected_states:
    raise SystemExit("RT1 Persona lifecycle state vocabulary drifted")

expected_forward = [
    ["DRAFT", "CAPTURED"],
    ["CAPTURED", "REVIEWED"],
    ["REVIEWED", "READY_FOR_PREVIEW"],
    ["READY_FOR_PREVIEW", "READY_FOR_PUBLICATION"],
    ["READY_FOR_PUBLICATION", "PUBLISHED"],
]
if contract.get("forward_transitions") != expected_forward:
    raise SystemExit("RT1 Persona forward lifecycle drifted")

expected_side = {
    ("PUBLISHED", "PAUSED"),
    ("PAUSED", "PUBLISHED"),
    ("PUBLISHED", "UNPUBLISHED"),
    ("PAUSED", "UNPUBLISHED"),
}
if {tuple(item) for item in contract.get("side_transitions", [])} != expected_side:
    raise SystemExit("RT1 Persona side lifecycle drifted")

for invariant in (
    "provider_success_cannot_advance_persona_lifecycle_directly",
    "preparation_job_ready_cannot_publish_persona",
    "review_required_before_preview_readiness",
    "preview_required_before_publication_readiness",
    "publication_requires_exact_current_persona_version",
    "publication_requires_required_modalities_ready_for_selected_configuration",
    "publication_requires_current_authorization_consent_and_rights",
    "publication_requires_no_stale_blocking_projection",
):
    if contract.get("gating_invariants", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 Persona lifecycle gate lost: {invariant}")

expected_correction_transitions = {
    "REVIEWED": "REVIEWED",
    "READY_FOR_PREVIEW": "REVIEWED",
    "READY_FOR_PUBLICATION": "REVIEWED",
    "PUBLISHED": "REVIEWED",
    "PAUSED": "REVIEWED",
}
if contract.get("correction_transitions") != expected_correction_transitions:
    raise SystemExit("RT1 Persona correction re-entry transitions drifted")

for invariant in (
    "correction_after_review_creates_new_persona_version",
    "affected_readiness_may_move_back_to_preparation_or_review",
    "unaffected_readiness_may_remain_valid_if_binding_still_current",
    "stale_preview_or_publication_binding_must_fail_closed",
    "prior_published_version_remains_immutable_history",
    "corrected_new_version_starts_reviewed",
    "repreview_required_before_republication",
):
    if contract.get("correction_and_invalidation", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 correction lifecycle invariant lost: {invariant}")

for invariant in (
    "pause_blocks_new_public_sessions",
    "unpublish_blocks_new_public_sessions",
    "pause_does_not_delete_private_persona",
    "unpublish_does_not_delete_private_persona",
    "resume_requires_current_valid_publication_binding",
):
    if contract.get("pause_and_unpublish", {}).get(invariant) is not True:
        raise SystemExit(f"RT1 pause/unpublish invariant lost: {invariant}")

expected_shortcuts = {
    ("DRAFT", "PUBLISHED"),
    ("CAPTURED", "PUBLISHED"),
    ("REVIEWED", "PUBLISHED"),
    ("READY_FOR_PREVIEW", "PUBLISHED"),
}
if {tuple(item) for item in contract.get("forbidden_shortcuts", [])} != expected_shortcuts:
    raise SystemExit("RT1 forbidden Persona lifecycle shortcuts drifted")

expected_fail_closed = {
    ("PROVIDER_RETURNS_READY_WHILE_PERSONA_NOT_REVIEWED", "DO_NOT_ADVANCE_BEYOND_CAPTURE_OR_REVIEW_GATE"),
    ("PUBLICATION_REQUEST_USES_STALE_PERSONA_VERSION", "DENY"),
    ("CORRECTION_INVALIDATES_REQUIRED_VIDEO_READINESS", "MOVE_AFFECTED_PATH_BACK_BEFORE_PUBLICATION"),
    ("PAUSED_PERSONA_RECEIVES_NEW_PUBLIC_SESSION_REQUEST", "DENY"),
    ("UNPUBLISHED_PERSONA_PRIVATE_OWNER_ACCESS", "ALLOW_IF_SEPARATELY_AUTHORIZED"),
}
actual_fail_closed = {
    (item.get("case"), item.get("decision"))
    for item in contract.get("fail_closed_examples", [])
}
if actual_fail_closed != expected_fail_closed:
    raise SystemExit("RT1 Persona lifecycle fail-closed matrix drifted")

# RT0 already owns experimental capture/review/readiness. Block only a distinct
# RT1 production lifecycle authority from appearing before RT0 exit.
surface_suffixes = {".rs", ".ts", ".tsx", ".js", ".html"}
forbidden_markers = (
    "Rt1PersonaLifecycle",
    "PersonaLifecycleRecord",
    "/api/persona/lifecycle",
    "/api/persona/publish-ready",
    'id="persona-lifecycle-manager"',
)
for source in CRATES.rglob("*"):
    if not source.is_file() or source.suffix not in surface_suffixes:
        continue
    text = source.read_text(encoding="utf-8")
    marker = next((item for item in forbidden_markers if item in text), None)
    if marker is not None:
        raise SystemExit(
            "production RT1 Persona lifecycle surface appeared while RT1 remains blocked "
            f"on RT0 exit: {source.relative_to(ROOT)} (marker={marker!r})"
        )

for required_spec_marker in (
    "Status:** `BLOCKED_ON_RT0_EXIT`",
    "Allowed pre-entry work:** `BOUNDED_FEASIBILITY_SPIKE_ONLY`",
    "DRAFT -> CAPTURED -> REVIEWED -> READY_FOR_PREVIEW -> READY_FOR_PUBLICATION -> PUBLISHED",
    "Persona lifecycle feasibility boundary",
    "Provider or preparation success must never advance Persona lifecycle by itself",
    "must remain contract-only and non-promoting",
):
    if required_spec_marker not in spec:
        raise SystemExit(f"RT1 Persona lifecycle spike lost ReleaseSpec guard: {required_spec_marker}")

print("rt1-persona-lifecycle-spike: PASS")
