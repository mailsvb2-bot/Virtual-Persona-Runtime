from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "rt1-answer-evidence-card-0.1.json"
RT1_SPEC = ROOT / "docs" / "releases" / "RT1_RELEASE_SPEC.md"
CRATES = ROOT / "crates"

contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
spec = RT1_SPEC.read_text(encoding="utf-8")

if contract.get("schema") != "rt1-answer-evidence-card-spike-0.1":
    raise SystemExit("RT1 Answer Evidence Card spike schema drifted")
if contract.get("maturity") != "RESEARCH_REQUIRED":
    raise SystemExit("RT1 Answer Evidence Card spike must remain RESEARCH_REQUIRED")
for flag in (
    "production",
    "answer_evidence_card_runtime_implemented",
    "owner_ui_implemented",
    "visitor_ui_implemented",
    "durable_trace_schema_implemented",
):
    if contract.get(flag) is not False:
        raise SystemExit(f"RT1 Answer Evidence Card spike must remain non-promoting: {flag}")

if contract.get("canonical_projection") != [
    "EXACT_TURN_EXECUTION_EVIDENCE",
    "DISCLOSURE_FILTER",
    "ANSWER_EVIDENCE_CARD",
    "HUMAN_READABLE_EXPLANATION",
]:
    raise SystemExit("Answer Evidence Card projection flow drifted")

expected_fields = {
    "SOURCES_USED",
    "OWNER_VERIFIED_MATERIAL",
    "SYSTEM_INFERENCE",
    "RESEARCH_DATE",
    "FRESHNESS",
    "UNCERTAINTY",
}
if set(contract.get("card_may_show", [])) != expected_fields:
    raise SystemExit("Answer Evidence Card visible evidence vocabulary drifted")

for section, required in {
    "trace_binding_invariants": (
        "binds_exact_answer_or_turn",
        "binds_exact_persona_version",
        "binds_actual_generation_evidence",
        "provider_failover_or_model_change_is_explicit_when_material",
        "card_is_projection_not_second_source_of_truth",
        "unsupported_post_hoc_explanation_forbidden",
        "missing_evidence_is_reported_as_unavailable_or_unknown",
    ),
    "epistemic_invariants": (
        "owner_verified_material_distinct_from_system_inference",
        "system_inference_not_presented_as_verified_owner_opinion",
        "simulated_content_not_presented_as_observed_fact",
        "freshness_and_uncertainty_remain_explicit_when_applicable",
    ),
    "privacy_and_authority": (
        "effective_audience_permissions_apply_to_card",
        "card_cannot_widen_source_disclosure",
        "private_source_payloads_are_not_exposed_by_default",
        "secrets_are_not_duplicated_into_card",
        "revocation_or_deletion_cannot_be_bypassed_by_historical_card",
        "source_restrictions_are_inherited_by_derived_explanation",
    ),
}.items():
    values = contract.get(section, {})
    for invariant in required:
        if values.get(invariant) is not True:
            raise SystemExit(f"Answer Evidence Card invariant lost: {section}.{invariant}")

voice = contract.get("voice_explanation", {})
if voice.get("question") != "Where do you know this from?":
    raise SystemExit("voice provenance question contract drifted")
for invariant in (
    "uses_same_real_trace",
    "human_readable",
    "does_not_require_hidden_chain_of_thought",
    "must_not_invent_missing_sources_or_rationale",
):
    if voice.get(invariant) is not True:
        raise SystemExit(f"voice provenance invariant lost: {invariant}")

expected_fail_closed = {
    ("TRACE_MISSING_FOR_ANSWER", "SHOW_EVIDENCE_UNAVAILABLE_NOT_INVENTED_EXPLANATION"),
    ("VISITOR_CARD_REQUESTS_OWNER_PRIVATE_SOURCE", "REDACT_OR_OMIT_BY_EFFECTIVE_AUDIENCE"),
    ("SYSTEM_INFERENCE_PRESENTED_AS_OWNER_VERIFIED_OPINION", "DENY_MISREPRESENTATION"),
    ("SOURCE_REVOKED_OR_DELETED_AFTER_TURN", "DO_NOT_REDISCLOSE_REVOKED_OR_DELETED_CONTENT"),
    ("VOICE_EXPLANATION_HAS_NO_TRACE_SUPPORT", "STATE_UNKNOWN_OR_UNAVAILABLE"),
}
actual_fail_closed = {
    (item.get("case"), item.get("decision"))
    for item in contract.get("fail_closed_examples", [])
}
if actual_fail_closed != expected_fail_closed:
    raise SystemExit("Answer Evidence Card fail-closed matrix drifted")

# While RT1 remains blocked on RT0 exit, the feasibility slice must not
# silently become a production Answer Evidence Card implementation through
# Rust, HTTP routes, or the user-reachable Owner Lab TypeScript/HTML surface.
production_surface_roots = (
    CRATES,
    ROOT / "crates" / "vpr-owner-lab" / "ui",
)
production_surface_suffixes = {".rs", ".ts", ".tsx", ".js", ".html"}
forbidden_surface_markers = (
    "AnswerEvidenceCard",
    "answerEvidenceCard",
    "answer_evidence_card",
    "answer-evidence-card",
    "/api/answer/evidence",
    "/api/evidence/answer",
    'id="answer-evidence"',
    'id="answer-evidence-card"',
    "Where do you know this from?",
)

scanned_paths: set[Path] = set()
for surface_root in production_surface_roots:
    for source in surface_root.rglob("*"):
        if (
            not source.is_file()
            or source.suffix not in production_surface_suffixes
            or source in scanned_paths
        ):
            continue
        scanned_paths.add(source)
        text = source.read_text(encoding="utf-8")
        marker = next((item for item in forbidden_surface_markers if item in text), None)
        if marker is not None:
            raise SystemExit(
                "production Answer Evidence Card surface appeared while RT1 remains blocked "
                f"on RT0 exit: {source.relative_to(ROOT)} (marker={marker!r})"
            )

for required_spec_marker in (
    "Status:** `BLOCKED_ON_RT0_EXIT`",
    "Allowed pre-entry work:** `BOUNDED_FEASIBILITY_SPIKE_ONLY`",
    "basic Answer Evidence Card",
    "Answer Evidence Card feasibility boundary",
    "real generation/evidence trace",
    "must not invent a post-hoc explanation",
    "must remain contract-only and non-promoting",
):
    if required_spec_marker not in spec:
        raise SystemExit(f"RT1 Answer Evidence Card spike lost ReleaseSpec guard: {required_spec_marker}")

print("rt1-answer-evidence-card-spike: PASS")
