#!/usr/bin/env python3
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
GATE = ROOT / "scripts" / "dependency_audit_gate.py"
FIXTURES = ROOT / "tests" / "security" / "fixtures"


def run(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(GATE), *args],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )


policy = run("--validate-policy-only")
if policy.returncode != 0:
    raise SystemExit(policy.stdout)

safe = run("--npm-json", str(FIXTURES / "npm-moderate.json"))
if safe.returncode != 0:
    raise SystemExit("moderate fixture must pass explicit high-severity policy:\n" + safe.stdout)

denied = run("--npm-json", str(FIXTURES / "npm-high.json"))
if denied.returncode == 0:
    raise SystemExit("high-severity fixture must be denied")
if "fixture-vulnerable-package:high" not in denied.stdout:
    raise SystemExit("high-severity fixture denial must identify the package")

print("dependency-audit-policy-tests: PASS")
