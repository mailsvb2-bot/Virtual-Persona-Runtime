from pathlib import Path
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[2]
domain_manifest = ROOT / "crates" / "vpr-domain" / "Cargo.toml"
domain_source = ROOT / "crates" / "vpr-domain" / "src"

manifest = tomllib.loads(domain_manifest.read_text(encoding="utf-8"))
dependencies = manifest.get("dependencies", {})
if dependencies:
    raise SystemExit(f"vpr-domain must remain provider/infrastructure free in RT0; found dependencies: {sorted(dependencies)}")

for path in domain_source.glob("*.rs"):
    text = path.read_text(encoding="utf-8")
    for forbidden in ("vpr_integration", "vpr_policy", "vpr_runtime"):
        if forbidden in text:
            raise SystemExit(f"forbidden inward dependency {forbidden!r} in {path.relative_to(ROOT)}")

print("architecture-boundaries: PASS")
sys.exit(0)
