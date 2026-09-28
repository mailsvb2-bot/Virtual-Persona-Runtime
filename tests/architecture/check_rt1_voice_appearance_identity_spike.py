from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "rt1-voice-appearance-identity-0.1.json"
RT1_SPEC = ROOT / "docs" / "releases" / "RT1_RELEASE_SPEC.md"
CRATES = ROOT / "crates"

contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
spec = RT1_SPEC.read_text(encoding="utf-8")

if contract.get("schema") != "rt1-voice-appearance-identity-spike-0.1":
    raise SystemExit("RT1 voice/appearance identity spike schema drifted")
if contract.get("maturity") != "RESEARCH_REQUIRED":
    raise SystemExit("RT1 voice/appearance identity spike must remain RESEARCH_REQUIRED")
for flag in (
    "production",
    "voice_identity_runtime_implemented",
    "appearance_identity_runtime_implemented",
    "owner_identity_ui_implemented",
):
    if contract.get(flag) is not False:
        raise SystemExit(f"RT1 voice/appearance identity spike must remain non-promoting: {flag}")

for invariant in (
    "persona_identity_not_llm_provider",
    "voice_identity_not_tts_provider_voice_id",
    "appearance_identity_not_avatar_provider_id",
    "external_ids_are_representations_or_bindings",
    "voice_change_not_identity_change",
    "appearance_change_not_identity_change",
    "appearance_change_not_personality_change",
):
    if contract.get("canonical_invariants", {}).get(invariant) is not True:
        raise SystemExit(f"identity invariant lost: {invariant}")

if set(contract.get("voice_identity", {}).get("operations", [])) != {
    "CREATE", "SELECT", "IMPORT", "CONNECT", "AUTO_ROUTE"
}:
    raise SystemExit("VoiceIdentity operation vocabulary drifted")

for invariant in (
    "requires_rights_or_consent_for_real_person",
    "reference_validation_required",
    "quality_evaluation_required",
    "versioned",
    "multiple_provider_or_language_representations_allowed",
    "provider_migration_may_preserve_logical_identity",
    "material_acoustic_change_visible_to_owner",
    "material_change_requires_reevaluation",
):
    if contract.get("voice_identity", {}).get(invariant) is not True:
        raise SystemExit(f"VoiceIdentity invariant lost: {invariant}")

appearance = contract.get("appearance_identity", {})
if set(appearance.get("operations", [])) != {"CREATE", "SELECT", "IMPORT", "CONNECT", "AUTO_ROUTE"}:
    raise SystemExit("AppearanceIdentity operation vocabulary drifted")
if set(appearance.get("supported_embodiment_classes", [])) != {
    "REAL_HUMAN", "STYLIZED_HUMAN", "ANIMAL", "NON_HUMAN_CHARACTER"
}:
    raise SystemExit("AppearanceIdentity embodiment classes drifted")
if set(appearance.get("maturity_states", [])) != {
    "NOT_TESTED", "EXPERIMENTAL", "SUPPORTED", "PRODUCTION_READY"
}:
    raise SystemExit("AppearanceIdentity maturity states drifted")
for invariant in (
    "real_person_requires_explicit_consent_scope",
    "provider_representation_does_not_own_identity",
    "material_identity_fidelity_change_visible_to_owner",
    "material_change_requires_reevaluation",
):
    if appearance.get(invariant) is not True:
        raise SystemExit(f"AppearanceIdentity invariant lost: {invariant}")

for invariant in (
    "binds_logical_identity",
    "binds_provider_and_model_version",
    "binds_rights_or_consent_state",
    "binds_quality_evidence_or_status",
    "secrets_not_part_of_canonical_identity",
    "representation_can_be_replaced_without_new_persona_identity",
):
    if contract.get("representation_binding", {}).get(invariant) is not True:
        raise SystemExit(f"representation binding invariant lost: {invariant}")

for invariant in (
    "compatibility_check_required",
    "candidate_representation_not_active_by_default",
    "quality_comparison_required_for_material_replacement",
    "rights_or_consent_rechecked",
    "owner_approval_required_when_material_identity_fidelity_changes",
    "activation_explicit",
    "rollback_point_required",
    "automatic_material_substitution_for_real_person_forbidden",
):
    if contract.get("migration_and_activation", {}).get(invariant) is not True:
        raise SystemExit(f"migration/activation invariant lost: {invariant}")

expected_fail_closed = {
    ("TTS_PROVIDER_VOICE_ID_CHANGES", "DO_NOT_CREATE_NEW_PERSONA_OR_RENAME_VOICE_IDENTITY"),
    ("AVATAR_PROVIDER_ID_CHANGES", "DO_NOT_CREATE_NEW_PERSONA_OR_RENAME_APPEARANCE_IDENTITY"),
    ("REAL_PERSON_VOICE_REPRESENTATION_LACKS_CONSENT", "DO_NOT_ACTIVATE"),
    ("MATERIAL_ACOUSTIC_OR_VISUAL_DELTA_WITHOUT_REQUIRED_OWNER_APPROVAL", "KEEP_CANDIDATE_INACTIVE"),
    ("PROVIDER_REPLACEMENT_INVALIDATES_QUALITY_CLAIM", "MARK_RELEVANT_EVALUATION_STALE_AND_REEVALUATE"),
}
actual_fail_closed = {
    (item.get("case"), item.get("decision"))
    for item in contract.get("fail_closed_examples", [])
}
if actual_fail_closed != expected_fail_closed:
    raise SystemExit("RT1 voice/appearance identity fail-closed matrix drifted")

surface_suffixes = {".rs", ".ts", ".tsx", ".js", ".html"}
forbidden_markers = (
    "struct VoiceIdentity",
    "struct AppearanceIdentity",
    "VoiceIdentityRecord",
    "AppearanceIdentityRecord",
    "/api/voice-identities",
    "/api/appearance-identities",
    'id="voice-identity-manager"',
    'id="appearance-identity-manager"',
)
for source in CRATES.rglob("*"):
    if not source.is_file() or source.suffix not in surface_suffixes:
        continue
    text = source.read_text(encoding="utf-8")
    marker = next((item for item in forbidden_markers if item in text), None)
    if marker is not None:
        raise SystemExit(
            "production RT1 voice/appearance identity surface appeared while RT1 remains blocked "
            f"on RT0 exit: {source.relative_to(ROOT)} (marker={marker!r})"
        )

for required_spec_marker in (
    "Status:** `BLOCKED_ON_RT0_EXIT`",
    "Allowed pre-entry work:** `BOUNDED_FEASIBILITY_SPIKE_ONLY`",
    "VoiceIdentity and AppearanceIdentity",
    "VoiceIdentity / AppearanceIdentity feasibility boundary",
    "VoiceIdentity != TTS provider voice ID",
    "AppearanceIdentity != avatar provider ID",
    "must remain contract-only and non-promoting",
):
    if required_spec_marker not in spec:
        raise SystemExit(f"RT1 voice/appearance identity spike lost ReleaseSpec guard: {required_spec_marker}")

print("rt1-voice-appearance-identity-spike: PASS")
