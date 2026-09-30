from __future__ import annotations

from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parents[2]
CI = ROOT / ".github" / "workflows" / "ci.yml"
TOOLCHAIN = ROOT / "rust-toolchain.toml"
DEPENDABOT = ROOT / ".github" / "dependabot.yml"

ci = CI.read_text(encoding="utf-8")
toolchain = tomllib.loads(TOOLCHAIN.read_text(encoding="utf-8"))
dependabot = DEPENDABOT.read_text(encoding="utf-8")

channel = toolchain.get("toolchain", {}).get("channel")
if channel != "1.88.0":
    raise SystemExit(f"Rust CI toolchain must remain pinned to declared MSRV 1.88.0, got {channel!r}")

components = set(toolchain.get("toolchain", {}).get("components", []))
if components != {"clippy", "rustfmt"}:
    raise SystemExit(f"Rust toolchain components drifted: {sorted(components)}")

for forbidden in ("ubuntu-latest", "windows-latest", "actions/checkout@v", "actions/setup-node@v"):
    if forbidden in ci:
        raise SystemExit(f"CI reproducibility regression: moving reference {forbidden!r} is forbidden")

if "node-version: 24.21.0" not in ci:
    raise SystemExit("CI Node runtime must remain pinned to 24.21.0")
if re.search(r"node-version:\s+24(?:\s|$)", ci):
    raise SystemExit("CI Node runtime must not use a moving major-only version")

for required in ("runs-on: ubuntu-24.04", "runs-on: windows-2022"):
    if required not in ci:
        raise SystemExit(f"CI must use pinned runner image: {required}")

rust_install = "rustup toolchain install 1.88.0 --profile minimal --component rustfmt --component clippy"
if ci.count(rust_install) != 2:
    raise SystemExit("CI must explicitly install the pinned Rust toolchain in both Linux and Windows Rust jobs")
if "rustup component add rustfmt clippy" in ci:
    raise SystemExit("CI must not rely on component-only rustup auto-install behavior")
for toolchain_step in re.findall(r"- name: Toolchain\n\s+run: \|\n(?P<body>(?:\s{10}.*\n?)+)", ci):
    if rust_install not in toolchain_step:
        raise SystemExit("every Rust Toolchain step must begin from explicit pinned installation")

action_refs = re.findall(r"uses:\s+(actions/(?:checkout|setup-node))@([^\s#]+)", ci)
if not action_refs:
    raise SystemExit("expected pinned first-party GitHub Actions references")
for action, ref in action_refs:
    if re.fullmatch(r"[0-9a-f]{40}", ref) is None:
        raise SystemExit(f"{action} must be pinned to a full commit SHA, got {ref!r}")

if "permissions:\n  contents: read" not in ci:
    raise SystemExit("CI token permissions must remain explicitly read-only")

for ecosystem in ("cargo", "npm", "github-actions"):
    if f'package-ecosystem: "{ecosystem}"' not in dependabot:
        raise SystemExit(f"Dependabot coverage missing for {ecosystem}")

print("ci-reproducibility: PASS")
