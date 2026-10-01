from pathlib import Path
import json

ROOT = Path(__file__).resolve().parents[2]
UI = ROOT / "crates" / "vpr-owner-lab" / "ui"
APP = (UI / "src" / "app.ts").read_text(encoding="utf-8")
SERVER = (ROOT / "crates" / "vpr-owner-lab" / "src" / "main.rs").read_text(encoding="utf-8")
FIXTURE_SERVER = (UI / "e2e" / "server.mjs").read_text(encoding="utf-8")
VENDOR_TEST = (UI / "e2e" / "vendor-livekit.spec.ts").read_text(encoding="utf-8")
VENDOR_SCRIPT = (UI / "scripts" / "vendor-livekit.mjs").read_text(encoding="utf-8")
PACKAGE = json.loads((UI / "package.json").read_text(encoding="utf-8"))
LOCK = json.loads((UI / "package-lock.json").read_text(encoding="utf-8"))

VERSION = "2.22.3"
ASSET = UI / "dist" / "vendor" / "livekit-client.umd.js"
DIGEST = UI / "dist" / "vendor" / "livekit-client.umd.js.sha256"
LICENSE = UI / "vendor" / "livekit-client.LICENSE"
NOTICE = UI / "vendor" / "livekit-client.NOTICE"

if "cdn.jsdelivr.net" in APP or "unpkg.com" in APP:
    raise SystemExit("Owner Lab runtime must not load executable LiveKit code from a CDN")
if 'const LIVEKIT_CLIENT_URL = "/vendor/livekit-client.umd.js";' not in APP:
    raise SystemExit("Owner Lab LiveKit loader must target the same-origin vendor route")
if "script.crossOrigin" in APP:
    raise SystemExit("same-origin LiveKit loader must not retain a CDN crossOrigin boundary")

script_directive = next(
    (item.strip() for item in SERVER.split('"') if "script-src" in item),
    "",
)
if "script-src 'self'" not in script_directive:
    raise SystemExit("Owner Lab CSP must allow same-origin scripts")
if "https://" in script_directive or "http://" in script_directive or "cdn.jsdelivr.net" in SERVER:
    raise SystemExit("Owner Lab script-src must not trust external executable origins")

if f'livekit-client' not in PACKAGE.get("devDependencies", {}):
    raise SystemExit("livekit-client must be explicitly pinned in package.json")
if PACKAGE["devDependencies"]["livekit-client"] != VERSION:
    raise SystemExit("livekit-client package.json version must be exact")

locked = LOCK.get("packages", {}).get("node_modules/livekit-client")
if not isinstance(locked, dict) or locked.get("version") != VERSION:
    raise SystemExit("package-lock must pin the exact LiveKit client version")
if not str(locked.get("integrity", "")).startswith("sha512-"):
    raise SystemExit("package-lock LiveKit entry must retain npm integrity evidence")

for required in (
    'const EXPECTED_VERSION = "2.22.3";',
    "node_modules/livekit-client/dist/livekit-client.umd.js",
    "createHash",
    "sha256",
):
    if required not in VENDOR_SCRIPT:
        raise SystemExit(f"LiveKit vendor generator drifted: {required}")

for required in (
    'include_str!("../ui/dist/vendor/livekit-client.umd.js")',
    '"/vendor/livekit-client.umd.js"',
):
    if required not in SERVER:
        raise SystemExit(f"Owner Lab server does not embed/serve self-hosted LiveKit: {required}")

if '["/vendor/livekit-client.umd.js", "dist/vendor/livekit-client.umd.js"]' not in FIXTURE_SERVER:
    raise SystemExit("browser fixture must serve the same self-hosted LiveKit artifact")
for required in (
    'request.get("/vendor/livekit-client.umd.js")',
    'page.addScriptTag({ url: "/vendor/livekit-client.umd.js" })',
    'expect(roomType).toBe("function")',
):
    if required not in VENDOR_TEST:
        raise SystemExit(f"browser self-hosted LiveKit proof missing: {required}")

if not ASSET.is_file() or ASSET.stat().st_size == 0:
    raise SystemExit("committed self-hosted LiveKit artifact is missing")
if not DIGEST.is_file():
    raise SystemExit("committed self-hosted LiveKit digest is missing")
if not LICENSE.is_file() or not NOTICE.is_file():
    raise SystemExit("LiveKit license/notice provenance is missing")

print("livekit-supply-chain: PASS")
