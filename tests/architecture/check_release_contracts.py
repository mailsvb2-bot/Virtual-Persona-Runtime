import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CATALOGUE = ROOT / "docs" / "capabilities" / "RT0.json"
SPEC = ROOT / "docs" / "releases" / "RT0_RELEASE_SPEC.md"
REASON_SOURCE = ROOT / "crates" / "vpr-domain" / "src" / "reason.rs"

ALLOWED_MATURITY = {
    "NOT_IMPLEMENTED",
    "RESEARCH_REQUIRED",
    "EXPERIMENTAL",
    "IMPLEMENTED",
    "USER_REACHABLE",
    "PRODUCTION_READY",
    "DEPRECATED",
}
REQUIRED_REASON_CODES = {
    "AUTH_REVOKED",
    "AUTH_EXPIRED",
    "AUTH_SCOPE_DENIED",
    "EGRESS_DENIED",
    "EGRESS_LOCAL_ONLY",
    "CONSENT_REQUIRED",
    "PROVIDER_UNAVAILABLE",
    "PROVIDER_RATE_LIMITED",
    "PROVIDER_TIMEOUT",
    "PREPARATION_FAILED",
    "TURN_CANCELLED",
    "INVALID_STATE_TRANSITION",
    "OWNER_ATTRIBUTION_UNVERIFIED",
    "BUDGET_EXHAUSTED",
    "INTERNAL_ERROR",
}

catalogue = json.loads(CATALOGUE.read_text(encoding="utf-8"))
spec_text = SPEC.read_text(encoding="utf-8")
ceiling_match = re.search(r"Maturity ceiling during RT0:\*\* `([A-Z_]+)`", spec_text)
if ceiling_match is None:
    raise SystemExit("RT0 ReleaseSpec is missing a parseable maturity ceiling")
maturity_ceiling = ceiling_match.group(1)
MATURITY_RANK = {
    "NOT_IMPLEMENTED": 0,
    "RESEARCH_REQUIRED": 1,
    "EXPERIMENTAL": 2,
    "IMPLEMENTED": 3,
    "USER_REACHABLE": 4,
    "PRODUCTION_READY": 5,
}
if maturity_ceiling not in MATURITY_RANK:
    raise SystemExit(f"unsupported RT0 maturity ceiling: {maturity_ceiling}")
if catalogue.get("release_train") != "RT0":
    raise SystemExit("RT0 capability catalogue has the wrong release_train")
if "version" in catalogue:
    raise SystemExit("RT0 capability catalogue must use catalogue_version only; duplicate top-level version is forbidden")
catalogue_version = catalogue.get("catalogue_version")
if not isinstance(catalogue_version, str) or re.fullmatch(r"\d+\.\d+\.\d+", catalogue_version) is None:
    raise SystemExit("RT0 capability catalogue must have a semver catalogue_version")

capabilities = catalogue.get("capabilities")
if not isinstance(capabilities, list) or not capabilities:
    raise SystemExit("RT0 capability catalogue must contain a non-empty capabilities list")

ids = []
for capability in capabilities:
    capability_id = capability.get("id")
    maturity = capability.get("maturity")
    user_reachable = capability.get("user_reachable")
    if not isinstance(capability_id, str) or not capability_id.strip():
        raise SystemExit(f"invalid capability id: {capability_id!r}")
    ids.append(capability_id)
    if maturity not in ALLOWED_MATURITY:
        raise SystemExit(f"invalid maturity {maturity!r} for {capability_id}")
    if maturity != "DEPRECATED" and MATURITY_RANK.get(maturity, 999) > MATURITY_RANK[maturity_ceiling]:
        raise SystemExit(
            f"{capability_id} exceeds active RT0 {maturity_ceiling} maturity ceiling: {maturity}"
        )
    if not isinstance(user_reachable, bool):
        raise SystemExit(f"user_reachable must be boolean for {capability_id}")
    if user_reachable and maturity not in {"USER_REACHABLE", "PRODUCTION_READY"}:
        raise SystemExit(
            f"{capability_id} cannot be user_reachable while maturity is {maturity}"
        )

if len(ids) != len(set(ids)):
    raise SystemExit("RT0 capability catalogue contains duplicate capability ids")


capability_by_id = {capability["id"]: capability for capability in capabilities}
required_capability_states = {
    "api.rt0_owner_capture_review": ("EXPERIMENTAL", False),
    "evaluation.rt0_exit_evidence_gate": ("EXPERIMENTAL", False),
    "evaluation.rt0_live_proof_preflight": ("EXPERIMENTAL", False),
    "evaluation.rt0_live_provider_probe": ("EXPERIMENTAL", False),
    "evaluation.rt0_owner_lab_session_evidence": ("EXPERIMENTAL", False),
    "evaluation.rt0_session_evidence_aggregate": ("EXPERIMENTAL", False),
    "evaluation.rt0_session_evidence_binding": ("EXPERIMENTAL", False),
    "evaluation.rt0_golden_set": ("NOT_IMPLEMENTED", False),
    "runtime.owner_lab_reviewed_owner_context": ("EXPERIMENTAL", False),
    "runtime.owner_lab_visitor_scope": ("EXPERIMENTAL", False),
    "ui.rt0_owner_capture_review": ("EXPERIMENTAL", False),
    "ui.rt0_visitor_test_session": ("EXPERIMENTAL", False),
    "ui.rt0_post_review_correction": ("EXPERIMENTAL", False),
    "provider.real_llm": ("NOT_IMPLEMENTED", False),
    "provider.real_stt": ("NOT_IMPLEMENTED", False),
    "provider.real_avatar": ("NOT_IMPLEMENTED", False),
    "release.rt0_exit_gate": ("NOT_IMPLEMENTED", False),
}
for capability_id, expected in required_capability_states.items():
    capability = capability_by_id.get(capability_id)
    if capability is None:
        raise SystemExit(f"RT0 capability catalogue is missing {capability_id}")
    actual = (capability.get("maturity"), capability.get("user_reachable"))
    if actual != expected:
        raise SystemExit(
            f"{capability_id} must remain {expected} until real exact-candidate evidence exists; got {actual}"
        )


OWNER_LAB_SRC = ROOT / "crates" / "vpr-owner-lab" / "src"
OWNER_LAB_UI = ROOT / "crates" / "vpr-owner-lab" / "ui" / "src" / "owner-capture.ts"
OWNER_LAB_APP = ROOT / "crates" / "vpr-owner-lab" / "ui" / "src" / "app.ts"
owner_capture_http = (OWNER_LAB_SRC / "http_owner_capture.rs").read_text(encoding="utf-8")
owner_lab_state = (OWNER_LAB_SRC / "state.rs").read_text(encoding="utf-8")
owner_context_source = (OWNER_LAB_SRC / "owner_context.rs").read_text(encoding="utf-8")
owner_capture_ui = OWNER_LAB_UI.read_text(encoding="utf-8")
for required_post_review_boundary in (
    "/api/persona/reviewed",
    "reviewed_owner_context_snapshot",
    "ReviewedOwnerContextSnapshot",
    "ReviewedOwnerClaimSnapshot",
):
    if required_post_review_boundary not in owner_capture_http + owner_lab_state + owner_context_source:
        raise SystemExit(
            f"Owner Lab post-review correction boundary missing {required_post_review_boundary}"
        )
owner_lab_main = (OWNER_LAB_SRC / "main.rs").read_text(encoding="utf-8")
if '(&Method::Get, "/api/persona/reviewed")' in owner_lab_main:
    raise SystemExit("reviewed owner claims must not be exposed through an unauthenticated GET route")
if '"/api/persona/reviewed"' not in owner_capture_ui:
    raise SystemExit("Owner Lab reviewed-claim UI must reload canonical current revisions after correction")
reviewed_snapshot_match = re.search(
    r"pub struct ReviewedOwnerContextSnapshot\s*\{.*?\n\}",
    owner_context_source,
    re.DOTALL,
)
if reviewed_snapshot_match is None:
    raise SystemExit("browser-facing reviewed snapshot type is missing")
if "previous_revisions" in reviewed_snapshot_match.group(0):
    raise SystemExit("browser-facing reviewed snapshot must not expose prior claim revisions")

owner_lab_voice = (OWNER_LAB_SRC / "state" / "voice.rs").read_text(encoding="utf-8")
owner_lab_app = OWNER_LAB_APP.read_text(encoding="utf-8")
for required_visitor_boundary in (
    "LabSessionAudience::Visitor",
    "start_visitor",
    "Rt0ReasonCode::AuthScopeDenied",
):
    if required_visitor_boundary not in owner_lab_state:
        raise SystemExit(f"Owner Lab visitor scope missing runtime boundary {required_visitor_boundary}")
if "VISITOR_PROMPT_PREFIX" not in owner_lab_voice:
    raise SystemExit("Owner Lab visitor turns must use a visitor-specific LLM context boundary")
if 'option[value="visitor"]' not in owner_lab_app or 'audience' not in owner_lab_app:
    raise SystemExit("Owner Lab browser must expose the visitor test-session selector")
if 'reviewed_owner_claims: if self.session_audience == Some(LabSessionAudience::Visitor)' not in owner_lab_state:
    raise SystemExit("visitor status must withhold reviewed owner claim-count metadata")

reason_source = REASON_SOURCE.read_text(encoding="utf-8")
for code in sorted(REQUIRED_REASON_CODES):
    if f"`{code}`" not in spec_text:
        raise SystemExit(f"RT0 ReleaseSpec is missing stable reason code {code}")
    if f'"{code}"' not in reason_source:
        raise SystemExit(f"runtime reason-code source is missing {code}")

print(
    f"release-contracts: PASS ({len(capabilities)} capabilities, "
    f"{len(REQUIRED_REASON_CODES)} reason codes)"
)
