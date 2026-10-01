from pathlib import Path
import json
import re

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "dependency-security.yml"
POLICY_PATH = ROOT / "security" / "dependency-audit-policy.json"
GATE = ROOT / "scripts" / "dependency_audit_gate.py"
FIXTURE = ROOT / "tests" / "security" / "fixtures" / "rustsec-vulnerable.Cargo.lock"

workflow = WORKFLOW.read_text(encoding="utf-8")
policy = json.loads(POLICY_PATH.read_text(encoding="utf-8"))
gate = GATE.read_text(encoding="utf-8")
fixture = FIXTURE.read_text(encoding="utf-8")

cargo = policy["cargo_audit"]
npm = policy["npm_audit"]
review = policy["dependency_review"]

required = (
    "cargo install cargo-audit --version 0.22.2 --locked",
    "cargo audit --file Cargo.lock",
    "cargo audit --no-fetch --file tests/security/fixtures/rustsec-vulnerable.Cargo.lock",
    'grep -q "RUSTSEC-2023-0071"',
    "npm audit --package-lock-only --ignore-scripts --json",
    "python3 scripts/dependency_audit_gate.py --npm-json /tmp/npm-audit.json",
    "actions/dependency-review-action@a1d282b36b6f3519aa1f3fc636f609c47dddb294",
    "fail-on-severity: high",
)
for value in required:
    if value not in workflow:
        raise SystemExit(f"dependency security workflow drifted: {value}")

for forbidden in (
    "cargo install cargo-audit\n",
    "cargo install cargo-audit --force",
    "actions/dependency-review-action@v",
    "npm install",
    "npm ci",
):
    if forbidden in workflow:
        raise SystemExit(f"dependency security workflow contains unpinned/script-capable path: {forbidden}")

action_refs = re.findall(r"uses:\s*([^\s]+)@([^\s#]+)", workflow)
for action, ref in action_refs:
    if not re.fullmatch(r"[0-9a-f]{40}", ref):
        raise SystemExit(f"dependency security action is not commit-pinned: {action}@{ref}")

if cargo != {
    "tool": "cargo-audit",
    "version": "0.22.2",
    "exceptions": [],
    "exception_budget": 0,
}:
    raise SystemExit("Rust dependency audit policy must remain exact and exception-free")

if npm.get("minimum_denied_severity") != "high":
    raise SystemExit("npm audit policy must deny high/critical vulnerabilities")
if npm.get("exceptions") != [] or npm.get("exception_budget") != 0:
    raise SystemExit("npm vulnerability exceptions require a separately reviewed policy extension")

if review.get("minimum_denied_severity") != "high":
    raise SystemExit("dependency-review policy must deny high/critical vulnerabilities")
if review.get("commit") != "a1d282b36b6f3519aa1f3fc636f609c47dddb294":
    raise SystemExit("dependency-review action pin drifted")

if 'name = "rsa"' not in fixture or 'version = "0.9.10"' not in fixture:
    raise SystemExit("RustSec denied-advisory fixture drifted")
if "RUSTSEC-2023-0071" not in workflow:
    raise SystemExit("RustSec fixture must assert its exact advisory")
if "minimum_denied_severity" not in gate or "SEVERITY_ORDER" not in gate:
    raise SystemExit("npm policy gate must retain explicit severity evaluation")

print("dependency-security-contract: PASS")
