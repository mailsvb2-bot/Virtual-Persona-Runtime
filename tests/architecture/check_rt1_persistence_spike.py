from __future__ import annotations

import json
from pathlib import Path
import tomllib

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "rt1-persistence-migration-0.1.json"
RT1_SPEC = ROOT / "docs" / "releases" / "RT1_RELEASE_SPEC.md"
OWNER_LAB_STATE = ROOT / "crates" / "vpr-owner-lab" / "src" / "state.rs"
PROFILE = ROOT / "crates" / "vpr-domain" / "src" / "profile.rs"
PREPARATION = ROOT / "crates" / "vpr-domain" / "src" / "preparation.rs"

contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
spec = RT1_SPEC.read_text(encoding="utf-8")
owner_lab = OWNER_LAB_STATE.read_text(encoding="utf-8")
profile = PROFILE.read_text(encoding="utf-8")
preparation = PREPARATION.read_text(encoding="utf-8")

if contract.get("schema") != "rt1-persistence-migration-spike-0.1":
    raise SystemExit("RT1 persistence spike schema drifted")
if contract.get("maturity") != "RESEARCH_REQUIRED":
    raise SystemExit("RT1 persistence spike must remain RESEARCH_REQUIRED")
for flag in ("production", "durable_database_implemented", "migration_executed"):
    if contract.get(flag) is not False:
        raise SystemExit(f"RT1 persistence spike must remain non-promoting: {flag}")

reality = contract.get("current_rt0_reality", {})
if reality.get("durable_application_store_present") is not False:
    raise SystemExit("research contract must not invent an RT0 durable application store")
if reality.get("owner_lab_state") != "IN_PROCESS_ONLY":
    raise SystemExit("RT0 Owner Lab state must remain described as process-local")
if reality.get("automatic_rt0_to_rt1_migration_possible") is not False:
    raise SystemExit("automatic RT0->RT1 migration must remain disabled")

migration = contract.get("migration_policy", {})
if migration.get("automatic_forward_migration") is not False:
    raise SystemExit("RT0 experimental state must not silently forward-migrate")
if migration.get("owner_reviewed_material") != "EXPLICIT_OWNER_CONTROLLED_BOOTSTRAP_ONLY":
    raise SystemExit("owner-reviewed RT0 material may enter RT1 only through explicit owner control")
if migration.get("release_evidence") != "ARCHIVE_AS_EVIDENCE_ONLY":
    raise SystemExit("RT0 release evidence must never become canonical product state")
if migration.get("unknown_or_ambiguous_state") != "DISCARD_FROM_PRODUCT_STATE_FAIL_CLOSED":
    raise SystemExit("ambiguous RT0 state must fail closed")

required_discard = {
    "rt0_session_id",
    "rt0_turn_id",
    "provider_session_id",
    "provider_stream_id",
    "webrtc_sdp",
    "webrtc_ice_credentials",
    "ephemeral_consent",
    "ephemeral_authority",
    "rt0_preparation_job_id",
    "rt0_provider_reachability_receipt",
    "rt0_browser_test_fixture",
    "rt0_release_evidence_artifact",
}
if set(contract.get("never_promote_to_canonical_rt1_state", [])) != required_discard:
    raise SystemExit("RT0 discard/non-promotion matrix drifted")

required_concepts = {
    "workspace_tenant_scope",
    "persona",
    "persona_version",
    "reviewed_owner_claim",
    "claim_revision_history",
    "voice_identity",
    "appearance_identity",
    "provider_neutral_representation_binding",
    "preparation_job",
    "publication_state",
    "consent_rights_state",
    "audit_record",
}
if set(contract.get("future_durable_concepts", [])) != required_concepts:
    raise SystemExit("future RT1 durable concept boundary drifted")

identity = contract.get("identity_invariants", {})
if identity.get("persona_identity") != ["PersonaId", "PersonaVersion"]:
    raise SystemExit("future persistence must bind canonical PersonaId + PersonaVersion")
for invariant in (
    "provider_identity_is_representation_only",
    "provider_replacement_is_not_persona_migration",
    "claim_history_must_not_be_squashed",
    "publication_points_to_exact_persona_version",
):
    if identity.get(invariant) is not True:
        raise SystemExit(f"persistence identity invariant lost: {invariant}")

mutations = contract.get("mutation_invariants", {})
for invariant in (
    "single_canonical_durable_commit_path",
    "auditable",
    "idempotent_when_retried",
    "persona_correction_advances_version_atomically_with_claim_revision",
    "stale_preparation_or_publication_projection_must_fail_closed",
    "pause_or_unpublish_does_not_delete_private_persona",
):
    if mutations.get(invariant) is not True:
        raise SystemExit(f"durable mutation invariant lost: {invariant}")

rollback = contract.get("rollback_invariants", {})
for invariant in (
    "no_identity_duplication",
    "reviewed_claim_history_preserved",
    "ambiguous_publication_state_defaults_private",
    "safe_roll_forward_allowed_when_reverse_migration_is_unsafe",
):
    if rollback.get(invariant) is not True:
        raise SystemExit(f"rollback invariant lost: {invariant}")

for required_source_marker in (
    "reviewed_owner_context: Option<ReviewedOwnerContext>",
    "session: Option<ActiveSession>",
    "avatar: Option<RealtimeAvatarHandle>",
    "session_counter: u64",
    "turn_counter: u64",
):
    if required_source_marker not in owner_lab:
        raise SystemExit(f"RT0 in-process state boundary changed: {required_source_marker}")

for required_profile_marker in (
    "previous_revisions: Vec<OwnerClaimRevision>",
    "pub struct PersonaProfile",
    "self.identity.apply_version(next_version)",
):
    if required_profile_marker not in profile:
        raise SystemExit(f"canonical profile revision boundary changed: {required_profile_marker}")

for required_preparation_marker in (
    "persona_id: PersonaId",
    "persona_version: PersonaVersion",
    "StalePersonaVersion",
    "StalePreparationJob",
):
    if required_preparation_marker not in preparation:
        raise SystemExit(f"preparation version-binding boundary changed: {required_preparation_marker}")

for required_spec_marker in (
    "Status:** `BLOCKED_ON_RT0_EXIT`",
    "Allowed pre-entry work:** `BOUNDED_FEASIBILITY_SPIKE_ONLY`",
    "silently migrate RT0 experimental state into a durable RT1 schema",
    "Every durable mutation affecting Persona, claim, permission, consent or publication state must be auditable and idempotent where retried.",
    "Provider replacement is never a Persona migration.",
):
    if required_spec_marker not in spec:
        raise SystemExit(f"RT1 persistence spike lost ReleaseSpec guard: {required_spec_marker}")

database_packages = {
    "sqlx",
    "rusqlite",
    "diesel",
    "tokio-postgres",
    "postgres",
    "sea-orm",
    "mongodb",
    "surrealdb",
}

def dependency_packages(manifest: dict) -> set[str]:
    found: set[str] = set()
    tables = ("dependencies", "dev-dependencies", "build-dependencies")
    for table_name in tables:
        table = manifest.get(table_name, {})
        for key, value in table.items():
            found.add(key)
            if isinstance(value, dict) and isinstance(value.get("package"), str):
                found.add(value["package"])
    for target in manifest.get("target", {}).values():
        for table_name in tables:
            table = target.get(table_name, {})
            for key, value in table.items():
                found.add(key)
                if isinstance(value, dict) and isinstance(value.get("package"), str):
                    found.add(value["package"])
    return found

for manifest_path in ROOT.rglob("Cargo.toml"):
    if "target" in manifest_path.parts:
        continue
    manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    forbidden = dependency_packages(manifest) & database_packages
    if forbidden:
        raise SystemExit(
            "database dependency introduced while RT1 durable persistence is still research-only: "
            + f"{manifest_path.relative_to(ROOT)} -> {sorted(forbidden)}"
        )

print("rt1-persistence-spike: PASS")
