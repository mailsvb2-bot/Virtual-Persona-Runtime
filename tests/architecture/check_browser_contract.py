from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
UI = ROOT / "crates" / "vpr-owner-lab" / "ui"
E2E = UI / "e2e" / "owner-journey.spec.ts"
BACKEND_E2E = UI / "e2e" / "backend-owner-journey.spec.ts"
BACKEND_CONFIG = UI / "playwright.backend.config.ts"
BACKEND_PROVIDER = UI / "e2e" / "backend-provider.mjs"
BACKEND_LAUNCHER = ROOT / "tests" / "e2e" / "run_owner_lab_backend.py"
CI = ROOT / ".github" / "workflows" / "ci.yml"

browser_e2e = E2E.read_text(encoding="utf-8")
backend_e2e = BACKEND_E2E.read_text(encoding="utf-8")
backend_config = BACKEND_CONFIG.read_text(encoding="utf-8")
backend_provider = BACKEND_PROVIDER.read_text(encoding="utf-8")
backend_launcher = BACKEND_LAUNCHER.read_text(encoding="utf-8")
default_config = (UI / "playwright.config.ts").read_text(encoding="utf-8")
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
    "AUTH_SCOPE_DENIED",
    "Basic backend-e2e-secret",
    "persona_version: 3",
    "/api/persona/reviewed",
    "/api/avatar/speak",
    "Отозвать доступ",
    "entry.method === \"DELETE\"",
):
    if required not in backend_e2e:
        raise SystemExit(f"Owner Lab backend browser proof missing: {required}")

for required in (
    "python3 ../../../tests/e2e/run_owner_lab_backend.py",
    "backend-provider.mjs",
):
    if required not in backend_config:
        raise SystemExit(f"Owner Lab backend browser config missing: {required}")

if 'testIgnore: "backend-owner-journey.spec.ts"' not in default_config:
    raise SystemExit("Default browser contract must exclude backend-integrated spec")

for required in ("VPR_DID_ENDPOINT", "VPR_DID_API_KEY", "cargo", "vpr-owner-lab"):
    if required not in backend_launcher:
        raise SystemExit(f"Owner Lab backend launcher missing: {required}")

for required in ("/agents/", "authorization", "session_id", "ice_servers"):
    if required not in backend_provider:
        raise SystemExit(f"Owner Lab backend provider fixture missing: {required}")
for required in (
    "python3 tests/architecture/check_browser_contract.py",
    "npx playwright install --with-deps chromium",
    "npm run test:e2e",
    "npm run test:e2e:backend",
    "dist/owner-capture.js",
):
    if required not in ci:
        raise SystemExit(f"Owner Lab CI missing browser-contract enforcement: {required}")

print("owner-lab-browser-contract: PASS")
