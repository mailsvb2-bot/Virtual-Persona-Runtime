#!/usr/bin/env python3
"""Fail-closed dependency vulnerability policy gate.

The real npm audit command produces JSON that is evaluated here instead of trusting npm's
process exit code alone. This keeps the severity policy explicit and makes network/tool failures
fail closed. Security exceptions are intentionally budgeted at zero until a reviewed exception
mechanism is introduced.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
POLICY_PATH = ROOT / "security" / "dependency-audit-policy.json"
SEVERITY_ORDER = {
    "info": 0,
    "low": 1,
    "moderate": 2,
    "high": 3,
    "critical": 4,
}


class GateError(RuntimeError):
    pass


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise GateError(f"cannot read valid JSON from {path}: {error}") from error
    if not isinstance(value, dict):
        raise GateError(f"{path} must contain a JSON object")
    return value


def validate_policy(policy: dict[str, Any]) -> None:
    if policy.get("schema_version") != 1:
        raise GateError("unsupported dependency audit policy schema")

    cargo = policy.get("cargo_audit")
    npm = policy.get("npm_audit")
    review = policy.get("dependency_review")
    for name, section in (("cargo_audit", cargo), ("npm_audit", npm), ("dependency_review", review)):
        if not isinstance(section, dict):
            raise GateError(f"missing policy section: {name}")

    if cargo.get("tool") != "cargo-audit" or cargo.get("version") != "0.22.2":
        raise GateError("cargo-audit must remain pinned to reviewed version 0.22.2")
    if review.get("action") != "actions/dependency-review-action":
        raise GateError("dependency review action identity drifted")
    commit = str(review.get("commit", ""))
    if len(commit) != 40 or any(ch not in "0123456789abcdef" for ch in commit):
        raise GateError("dependency review action must be pinned to a full commit SHA")

    for name, section in (("cargo_audit", cargo), ("npm_audit", npm)):
        exceptions = section.get("exceptions")
        budget = section.get("exception_budget")
        if not isinstance(exceptions, list) or not isinstance(budget, int) or budget < 0:
            raise GateError(f"{name} exception policy is malformed")
        if len(exceptions) > budget:
            raise GateError(f"{name} exceptions exceed the explicit reviewed budget")
        for item in exceptions:
            if not isinstance(item, dict):
                raise GateError(f"{name} exception must be an object")
            advisory = str(item.get("advisory", "")).strip()
            rationale = str(item.get("rationale", "")).strip()
            expires = str(item.get("expires", "")).strip()
            if not advisory or len(rationale) < 20:
                raise GateError(f"{name} exception requires advisory id and substantive rationale")
            try:
                expiry = dt.date.fromisoformat(expires)
            except ValueError as error:
                raise GateError(f"{name} exception expiry must be YYYY-MM-DD") from error
            today = dt.datetime.now(dt.timezone.utc).date()
            if expiry < today:
                raise GateError(f"{name} exception expired: {advisory}")
            if expiry > today + dt.timedelta(days=90):
                raise GateError(f"{name} exception exceeds 90-day maximum: {advisory}")

    npm_severity = str(npm.get("minimum_denied_severity", ""))
    review_severity = str(review.get("minimum_denied_severity", ""))
    if npm_severity not in SEVERITY_ORDER or review_severity not in SEVERITY_ORDER:
        raise GateError("unknown dependency severity threshold")
    if SEVERITY_ORDER[npm_severity] < SEVERITY_ORDER["high"]:
        raise GateError("npm vulnerability threshold may not be weaker than high")
    if SEVERITY_ORDER[review_severity] < SEVERITY_ORDER["high"]:
        raise GateError("dependency-review threshold may not be weaker than high")


def audit_npm(report: dict[str, Any], policy: dict[str, Any]) -> None:
    if report.get("error"):
        raise GateError(f"npm audit returned an error payload: {report['error']}")
    vulnerabilities = report.get("vulnerabilities")
    if not isinstance(vulnerabilities, dict):
        raise GateError("npm audit report is missing vulnerabilities object")

    threshold = SEVERITY_ORDER[str(policy["npm_audit"]["minimum_denied_severity"])]
    denied: list[str] = []
    for package, entry in vulnerabilities.items():
        if not isinstance(entry, dict):
            raise GateError(f"npm audit vulnerability entry is malformed: {package}")
        severity = str(entry.get("severity", "")).lower()
        if severity not in SEVERITY_ORDER:
            raise GateError(f"npm audit returned unknown severity for {package}: {severity}")
        if SEVERITY_ORDER[severity] >= threshold:
            denied.append(f"{package}:{severity}")

    if denied:
        raise GateError("denied npm vulnerabilities: " + ", ".join(sorted(denied)))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--npm-json", type=Path)
    parser.add_argument("--validate-policy-only", action="store_true")
    args = parser.parse_args()

    try:
        policy = load_json(POLICY_PATH)
        validate_policy(policy)
        if args.npm_json is not None:
            audit_npm(load_json(args.npm_json), policy)
        elif not args.validate_policy_only:
            raise GateError("one gate mode must be selected")
    except GateError as error:
        print(f"dependency-audit-gate: FAIL: {error}")
        return 1

    print("dependency-audit-gate: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
