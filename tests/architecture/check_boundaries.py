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

DEPENDENCY_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")

def all_dependency_names(manifest):
    names = set()
    for table in DEPENDENCY_TABLES:
        names.update(manifest.get(table, {}))
    for target in manifest.get("target", {}).values():
        for table in DEPENDENCY_TABLES:
            names.update(target.get(table, {}))
    return names

_target_probe = {
    "target": {
        "cfg(target_os = \"windows\")": {
            "dependencies": {"vpr-runtime": {"path": "../vpr-runtime"}}
        }
    }
}
if "vpr-runtime" not in all_dependency_names(_target_probe):
    raise SystemExit("architecture checker failed its target-specific dependency self-test")

for crate, allowed in allowed_internal_dependencies.items():
    manifest_path = CRATES / crate / "Cargo.toml"
    manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    dependencies = all_dependency_names(manifest)
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
if all_dependency_names(domain_manifest):
    raise SystemExit("vpr-domain must remain dependency-free in RT0, including target/dev/build dependencies")

for path in (CRATES / "vpr-domain" / "src").glob("*.rs"):
    text = path.read_text(encoding="utf-8")
    for forbidden in ("vpr_integration", "vpr_policy", "vpr_runtime"):
        if forbidden in text:
            raise SystemExit(
                f"forbidden inward dependency {forbidden!r} in {path.relative_to(ROOT)}"
            )

# Canonical mutable runtime identities must not regain split-brain clone/permit escape hatches.
runtime_source = (CRATES / "vpr-runtime" / "src" / "lib.rs").read_text(encoding="utf-8")
if "#[derive(Debug, Clone)]\npub struct ActiveTurn" in runtime_source:
    raise SystemExit("ActiveTurn must remain non-cloneable canonical mutable state")
if "pub struct ProviderExecutionPermit" in runtime_source:
    raise SystemExit("provider execution permits must remain internal to runtime execution methods")
if "pub fn begin_external_provider_call" in runtime_source:
    raise SystemExit("raw provider-call permit issuance must not be public")

print("architecture-boundaries: PASS")
sys.exit(0)
