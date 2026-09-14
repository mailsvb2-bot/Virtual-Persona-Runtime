from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
E2E = ROOT / "crates" / "vpr-owner-lab" / "ui" / "e2e" / "owner-journey.spec.ts"
CI = ROOT / ".github" / "workflows" / "ci.yml"

browser_e2e = E2E.read_text(encoding="utf-8")
ci = CI.read_text(encoding="utf-8")

for required in (
    "/api/persona/reviewed",
    "/api/session/revoke",
    'selectOption("visitor")',
    "startAudiences",
    "x-vpr-csrf",
    "active visitor recovery",
):
    if required not in browser_e2e:
        raise SystemExit(f"Owner Lab browser contract missing required proof: {required}")

for required in (
    "python3 tests/architecture/check_browser_contract.py",
    "npx playwright install --with-deps chromium",
    "npm run test:e2e",
    "dist/owner-capture.js",
):
    if required not in ci:
        raise SystemExit(f"Owner Lab CI missing browser-contract enforcement: {required}")

print("owner-lab-browser-contract: PASS")
