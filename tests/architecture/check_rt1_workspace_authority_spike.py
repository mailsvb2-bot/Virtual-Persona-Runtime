from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "rt1-workspace-authority-0.1.json"
RT1_SPEC = ROOT / "docs" / "releases" / "RT1_RELEASE_SPEC.md"
POLICY = ROOT / "crates" / "vpr-policy" / "src" / "lib.rs"
DOMAIN_IDS = ROOT / "crates" / "vpr-domain" / "src" / "ids.rs"
OWNER_LAB_STATE = ROOT / "crates" / "vpr-owner-lab" / "src" / "state.rs"
PUBLICATION = ROOT / "contracts" / "rt1-publication-boundary-0.1.json"

contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
spec = RT1_SPEC.read_text(encoding="utf-8")
policy = POLICY.read_text(encoding="utf-8")
domain_ids = DOMAIN_IDS.read_text(encoding="utf-8")
owner_lab = OWNER_LAB_STATE.read_text(encoding="utf-8")
publication = json.loads(PUBLICATION.read_text(encoding="utf-8"))

if contract.get("schema") != "rt1-workspace-authority-spike-0.1":
    raise SystemExit("RT1 workspace authority spike schema drifted")
if contract.get("maturity") != "RESEARCH_REQUIRED":
    raise SystemExit("RT1 workspace authority spike must remain RESEARCH_REQUIRED")
for flag in (
    "production",
    "workspace_identity_implemented",
    "account_system_implemented",
    "public_auth_implemented",
):
    if contract.get(flag) is not False:
        raise SystemExit(f"RT1 workspace spike must remain non-promoting: {flag}")

tenant = contract.get("first_rt1_tenant_model", {})
if tenant.get("tenant_boundary") != "WORKSPACE":
    raise SystemExit("first RT1 tenant boundary must remain one workspace")
if tenant.get("future_identity") != "WorkspaceId":
    raise SystemExit("future tenant identity must remain an explicit WorkspaceId concept")
for invariant in (
    "persona_belongs_to_exactly_one_workspace",
    "workspace_identity_distinct_from_persona_identity",
    "workspace_identity_distinct_from_provider_identity",
    "workspace_identity_not_derived_from_publication_token",
):
    if tenant.get(invariant) is not True:
        raise SystemExit(f"workspace identity invariant lost: {invariant}")

authorization = contract.get("authorization_model", {})
if authorization.get("canonical_permission_composition") != "EffectiveAuthority::compose":
    raise SystemExit("workspace spike must reuse canonical EffectiveAuthority composition")
for invariant in (
    "workspace_match_is_resource_boundary",
    "workspace_id_must_not_be_encoded_into_authority_scope",
    "explicit_deny_wins",
    "revoked_or_expired_authority_fails_closed",
    "workspace_match_cannot_widen_authority",
    "egress_policy_remains_additional_required_gate",
):
    if authorization.get(invariant) is not True:
        raise SystemExit(f"workspace authorization invariant lost: {invariant}")

principals = contract.get("principal_scopes", {})
owner = principals.get("owner", {})
visitor = principals.get("public_visitor", {})
for required in ("authenticated", "matching_workspace_required", "may_use_owner_routes", "may_preview", "may_publish_control"):
    if owner.get(required) is not True:
        raise SystemExit(f"owner workspace contract lost: {required}")
if visitor.get("authenticated_workspace_member") is not False:
    raise SystemExit("public visitor must not become a workspace member")
if visitor.get("matching_workspace_membership_required") is not False:
    raise SystemExit("public visitor bootstrap is publication-bound, not workspace-membership-bound")
if visitor.get("publication_bound") is not True:
    raise SystemExit("public visitor must remain publication-bound")
for forbidden in ("may_use_owner_routes", "may_read_owner_private_material", "may_mutate_persona"):
    if visitor.get(forbidden) is not False:
        raise SystemExit(f"public visitor gained forbidden owner capability: {forbidden}")

public_link = contract.get("public_link_invariants", {})
for false_invariant in ("is_owner_credential", "grants_workspace_membership", "grants_owner_authority"):
    if public_link.get(false_invariant) is not False:
        raise SystemExit(f"public link must not become owner authority: {false_invariant}")
for true_invariant in (
    "resolves_only_publication_boundary",
    "cannot_override_pause_or_unpublish",
    "cannot_override_stale_persona_version",
):
    if public_link.get(true_invariant) is not True:
        raise SystemExit(f"public link safety invariant lost: {true_invariant}")

required_examples = {
    ("OWNER_WRONG_WORKSPACE", "DENY_BEFORE_PROVIDER_EGRESS"),
    ("VISITOR_CALLS_OWNER_ROUTE", "DENY_BEFORE_PROVIDER_EGRESS"),
    ("PUBLIC_LINK_ON_PAUSED_PUBLICATION", "DENY_BEFORE_PROVIDER_EGRESS"),
    ("OWNER_MATCHING_WORKSPACE_BUT_SCOPE_DENIED", "DENY_BEFORE_PROVIDER_EGRESS"),
    (
        "OWNER_MATCHING_WORKSPACE_AND_SCOPE_ALLOWED",
        "CONTINUE_TO_NORMAL_POLICY_CONSENT_EGRESS_GATES",
    ),
}
actual_examples = {
    (item.get("case"), item.get("decision"))
    for item in contract.get("fail_closed_examples", [])
}
if actual_examples != required_examples:
    raise SystemExit("workspace authorization fail-closed matrix drifted")

for required_policy_marker in (
    "pub struct AuthorityScope(String);",
    "pub struct EffectiveAuthority",
    "pub fn compose(layers: &[AuthorityLayer]) -> Self",
    "allowed = allowed.intersection(&layer.allowed).cloned().collect();",
    "allowed.retain(|scope| !denied.contains(scope));",
    "pub struct AuthorizationState",
    "AuthorizationValidityError::StaleEpoch",
    "AuthorizationValidityError::Revoked",
    "AuthorizationValidityError::Expired",
    "pub const fn decide_egress",
):
    if required_policy_marker not in policy:
        raise SystemExit(f"canonical policy boundary changed: {required_policy_marker}")

if "canonical_id!(WorkspaceId);" in domain_ids or "canonical_id!(TenantId);" in domain_ids:
    raise SystemExit(
        "workspace/tenant production identity appeared while RT1 workspace contract is still research-only"
    )

for required_owner_lab_marker in (
    "pub enum LabSessionAudience",
    "Owner",
    "Visitor",
    "AuthScopeDenied",
):
    if required_owner_lab_marker not in owner_lab:
        raise SystemExit(f"RT0 owner/visitor scope boundary changed: {required_owner_lab_marker}")

public_bootstrap = publication.get("bootstrap", {}).get("public_visitor", {})
preview_bootstrap = publication.get("bootstrap", {}).get("preview", {})
if public_bootstrap.get("scope") != "PUBLIC_VISITOR":
    raise SystemExit("workspace spike must preserve publication-bound PUBLIC_VISITOR scope")
if preview_bootstrap.get("scope") != "OWNER_PREVIEW":
    raise SystemExit("workspace spike must preserve distinct OWNER_PREVIEW scope")
if preview_bootstrap.get("grants_public_authority") is not False:
    raise SystemExit("owner preview must not grant public authority")

for required_spec_marker in (
    "owner account/workspace basics with tenant-scoped authorization",
    "owner routes are tenant-scoped",
    "visitor preview and public visitor scopes never receive owner-private material unless explicitly audience-authorized",
    "one canonical policy/authorization model",
    "RT1 MUST NOT introduce a separate “owner product brain”",
):
    if required_spec_marker not in spec:
        raise SystemExit(f"RT1 workspace spike lost ReleaseSpec guard: {required_spec_marker}")

print("rt1-workspace-authority-spike: PASS")
