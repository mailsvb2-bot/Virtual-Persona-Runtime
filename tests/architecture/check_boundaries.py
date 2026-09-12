from pathlib import Path
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[2]
CRATES = ROOT / "crates"

allowed_internal_dependencies = {
    "vpr-domain": set(),
    "vpr-policy": {"vpr-domain"},
    "vpr-integration": {"vpr-domain"},
    "vpr-runtime": {"vpr-domain", "vpr-policy", "vpr-integration"},
}

for crate, allowed in allowed_internal_dependencies.items():
    manifest_path = CRATES / crate / "Cargo.toml"
    manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    dependencies = set(manifest.get("dependencies", {}))
    internal = {name for name in dependencies if name.startswith("vpr-")}
    forbidden = internal - allowed
    if forbidden:
        raise SystemExit(
            f"{crate} has forbidden inward/cyclic VPR dependencies: {sorted(forbidden)}"
        )

# The canonical domain must stay provider/infrastructure free, including third-party crates.
domain_manifest = tomllib.loads(
    (CRATES / "vpr-domain" / "Cargo.toml").read_text(encoding="utf-8")
)
if domain_manifest.get("dependencies", {}):
    raise SystemExit("vpr-domain must remain dependency-free in RT0")

for path in (CRATES / "vpr-domain" / "src").glob("*.rs"):
    text = path.read_text(encoding="utf-8")
    for forbidden in ("vpr_integration", "vpr_policy", "vpr_runtime"):
        if forbidden in text:
            raise SystemExit(
                f"forbidden inward dependency {forbidden!r} in {path.relative_to(ROOT)}"
            )

print("architecture-boundaries: PASS")
sys.exit(0)
