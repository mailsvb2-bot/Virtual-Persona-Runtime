from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "rt1-publication-boundary-0.1.json"
RT1_SPEC = ROOT / "docs" / "releases" / "RT1_RELEASE_SPEC.md"
DOMAIN_IDS = ROOT / "crates" / "vpr-domain" / "src" / "ids.rs"
DOMAIN_PERSONA = ROOT / "crates" / "vpr-domain" / "src" / "persona.rs"

contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
spec = RT1_SPEC.read_text(encoding="utf-8")
ids = DOMAIN_IDS.read_text(encoding="utf-8")
persona = DOMAIN_PERSONA.read_text(encoding="utf-8")

if contract.get("schema") != "rt1-publication-boundary-spike-0.1":
    raise SystemExit("RT1 publication spike schema drifted")
if contract.get("maturity") != "RESEARCH_REQUIRED":
    raise SystemExit("pre-entry publication spike must remain RESEARCH_REQUIRED")
for forbidden_promotion in ("production", "public_endpoint_exposed", "durable_state_implemented"):
    if contract.get(forbidden_promotion) is not False:
        raise SystemExit(f"RT1 publication spike must remain non-promoting: {forbidden_promotion}")

identity = contract.get("publication_identity", {})
if identity.get("fields") != ["publication_id", "persona_id", "persona_version"]:
    raise SystemExit("publication identity must stay a narrow pointer to canonical Persona identity/version")
if identity.get("canonical_persona_reference") != ["PersonaId", "PersonaVersion"]:
    raise SystemExit("publication boundary must reference canonical PersonaId + PersonaVersion")
if identity.get("copies_persona_truth") is not False:
    raise SystemExit("publication boundary must not copy Persona truth")
if identity.get("contains_provider_identity") is not False:
    raise SystemExit("publication identity must remain provider-neutral")

if "canonical_id!(PersonaId);" not in ids:
    raise SystemExit("canonical PersonaId is missing from vpr-domain")
if "pub struct PersonaVersion(u64);" not in persona:
    raise SystemExit("canonical PersonaVersion is missing from vpr-domain")

for field in contract.get("forbidden_identity_fields", []):
    if field in identity.get("fields", []):
        raise SystemExit(f"provider/representation identity leaked into publication identity: {field}")

expected_states = ["PRIVATE", "PREVIEWABLE", "PUBLISHED", "PAUSED", "UNPUBLISHED"]
if contract.get("states") != expected_states:
    raise SystemExit("publication state vocabulary drifted")

expected_transitions = {
    ("PRIVATE", "PREVIEWABLE"),
    ("PREVIEWABLE", "PUBLISHED"),
    ("PUBLISHED", "PAUSED"),
    ("PAUSED", "PUBLISHED"),
    ("PUBLISHED", "UNPUBLISHED"),
    ("PAUSED", "UNPUBLISHED"),
}
actual_transitions = {tuple(item) for item in contract.get("transitions", [])}
if actual_transitions != expected_transitions:
    raise SystemExit("publication transition contract drifted")

bootstrap = contract.get("bootstrap", {})
preview = bootstrap.get("preview", {})
public = bootstrap.get("public_visitor", {})
if preview.get("scope") != "OWNER_PREVIEW" or public.get("scope") != "PUBLIC_VISITOR":
    raise SystemExit("preview and public visitor scopes must remain distinct")
if preview.get("grants_public_authority") is not False:
    raise SystemExit("owner preview must never grant public authority")
if preview.get("requires_owner_authority") is not True:
    raise SystemExit("owner preview must require owner authority")
if public.get("allowed_states") != ["PUBLISHED"]:
    raise SystemExit("new public visitor bootstrap must be allowed only while PUBLISHED")
if public.get("requires_exact_persona_version") is not True:
    raise SystemExit("public bootstrap must bind to exact canonical PersonaVersion")
if public.get("deny_on_stale_persona_version") is not True:
    raise SystemExit("stale public projection must fail closed")
if public.get("deny_on_ambiguous_state") is not True:
    raise SystemExit("ambiguous publication state must fail closed")

examples = contract.get("fail_closed_examples", [])
required_examples = {
    ("PAUSED", "PUBLIC_VISITOR", None, "DENY"),
    ("UNPUBLISHED", "PUBLIC_VISITOR", None, "DENY"),
    ("PUBLISHED", "PUBLIC_VISITOR", False, "DENY"),
    ("PUBLISHED", "PUBLIC_VISITOR", True, "ALLOW"),
}
actual_examples = {
    (
        item.get("state"),
        item.get("scope"),
        item.get("persona_version_match"),
        item.get("decision"),
    )
    for item in examples
}
if actual_examples != required_examples:
    raise SystemExit("publication bootstrap fail-closed examples are incomplete")

for required_spec_marker in (
    "Status:** `BLOCKED_ON_RT0_EXIT`",
    "Allowed pre-entry work:** `BOUNDED_FEASIBILITY_SPIKE_ONLY`",
    "The preferred first spike is contract-level validation of the publication boundary",
    "must remain non-production, non-public and non-promoting",
):
    if required_spec_marker not in spec:
        raise SystemExit(f"RT1 publication spike lost pre-entry guard: {required_spec_marker}")

print("rt1-publication-spike: PASS")
