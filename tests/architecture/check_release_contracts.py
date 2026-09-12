import json
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
if catalogue.get("release_train") != "RT0":
    raise SystemExit("RT0 capability catalogue has the wrong release_train")

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
    if not isinstance(user_reachable, bool):
        raise SystemExit(f"user_reachable must be boolean for {capability_id}")
    if user_reachable and maturity not in {"USER_REACHABLE", "PRODUCTION_READY"}:
        raise SystemExit(
            f"{capability_id} cannot be user_reachable while maturity is {maturity}"
        )

if len(ids) != len(set(ids)):
    raise SystemExit("RT0 capability catalogue contains duplicate capability ids")

spec_text = SPEC.read_text(encoding="utf-8")
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
