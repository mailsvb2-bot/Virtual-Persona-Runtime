from pathlib import Path
import re
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[2]
CRATES = ROOT / "crates"

allowed_internal_dependencies = {
    "vpr-domain": set(),
    "vpr-policy": {"vpr-domain"},
    "vpr-integration": {"vpr-domain"},
    "vpr-capture": {"vpr-domain"},
    "vpr-runtime": {"vpr-domain", "vpr-policy", "vpr-integration"},
    "vpr-provider-openai-compatible": {"vpr-integration"},
    "vpr-provider-openai-transcription": {"vpr-integration"},
    "vpr-provider-deepgram-stt": {"vpr-integration"},
    "vpr-provider-anthropic": {"vpr-integration"},
    "vpr-provider-gemini": {"vpr-integration"},
    "vpr-rt0-smoke": {
        "vpr-domain",
        "vpr-integration",
        "vpr-policy",
        "vpr-provider-anthropic",
        "vpr-provider-gemini",
        "vpr-provider-openai-compatible",
        "vpr-provider-openai-transcription",
        "vpr-provider-deepgram-stt",
        "vpr-runtime",
    },
}

DEPENDENCY_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")
WORKSPACE_MANIFEST = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
WORKSPACE_DEPENDENCIES = WORKSPACE_MANIFEST.get("workspace", {}).get("dependencies", {})

def dependency_names(table, workspace_dependencies):
    names = set()
    for key, spec in table.items():
        names.add(key)
        if isinstance(spec, dict) and isinstance(spec.get("package"), str):
            names.add(spec["package"])
        if isinstance(spec, dict) and spec.get("workspace") is True:
            inherited = workspace_dependencies.get(key, {})
            if isinstance(inherited, dict) and isinstance(inherited.get("package"), str):
                names.add(inherited["package"])
    return names

def all_dependency_names(manifest, workspace_dependencies=WORKSPACE_DEPENDENCIES):
    names = set()
    for table in DEPENDENCY_TABLES:
        names.update(dependency_names(manifest.get(table, {}), workspace_dependencies))
    for target in manifest.get("target", {}).values():
        for table in DEPENDENCY_TABLES:
            names.update(dependency_names(target.get(table, {}), workspace_dependencies))
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

_renamed_probe = {
    "dependencies": {
        "policy": {"package": "vpr-policy", "path": "../vpr-policy"}
    }
}
if "vpr-policy" not in all_dependency_names(_renamed_probe):
    raise SystemExit("architecture checker failed its renamed-dependency self-test")

_workspace_inherited_probe = {
    "dependencies": {"policy": {"workspace": True}}
}
_workspace_catalog_probe = {
    "policy": {"package": "vpr-policy", "path": "crates/vpr-policy"}
}
if "vpr-policy" not in all_dependency_names(_workspace_inherited_probe, _workspace_catalog_probe):
    raise SystemExit("architecture checker failed its workspace-inherited dependency self-test")

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

# The canonical domain must stay provider/infrastructure/workflow free, including third-party crates.
domain_manifest = tomllib.loads(
    (CRATES / "vpr-domain" / "Cargo.toml").read_text(encoding="utf-8")
)
if all_dependency_names(domain_manifest):
    raise SystemExit("vpr-domain must remain dependency-free in RT0, including target/dev/build dependencies")

for path in (CRATES / "vpr-domain" / "src").glob("*.rs"):
    text = path.read_text(encoding="utf-8")
    for forbidden in ("vpr_capture", "vpr_integration", "vpr_policy", "vpr_runtime"):
        if forbidden in text:
            raise SystemExit(
                f"forbidden inward dependency {forbidden!r} in {path.relative_to(ROOT)}"
            )

# The mutable canonical Persona profile must not split into independently mutable clones.
profile_source = (CRATES / "vpr-domain" / "src" / "profile.rs").read_text(encoding="utf-8")
profile_derive = re.search(
    r"#\[derive\(([^)]*)\)\]\s*pub struct PersonaProfile\b",
    profile_source,
    re.MULTILINE,
)
if profile_derive is not None and any(
    item.strip() == "Clone" for item in profile_derive.group(1).split(",")
):
    raise SystemExit("PersonaProfile must remain non-cloneable canonical mutable state")

# Guided capture may orchestrate canonical Persona state, but it must not become another provider/runtime brain.
capture_src = CRATES / "vpr-capture" / "src"
for path in capture_src.rglob("*.rs"):
    text = path.read_text(encoding="utf-8")
    for forbidden in ("vpr_integration", "vpr_policy", "vpr_runtime"):
        if forbidden in text:
            raise SystemExit(
                f"guided capture has forbidden runtime/provider dependency {forbidden!r} in {path.relative_to(ROOT)}"
            )

# Canonical mutable runtime identities must not regain split-brain clone/permit escape hatches.
runtime_src = CRATES / "vpr-runtime" / "src"
turn_source = (runtime_src / "turn.rs").read_text(encoding="utf-8")
provider_source = (runtime_src / "provider.rs").read_text(encoding="utf-8")
cancellation_source = (runtime_src / "cancellation.rs").read_text(encoding="utf-8")
if "#[derive(Debug, Clone)]\npub struct ActiveTurn" in turn_source:
    raise SystemExit("ActiveTurn must remain non-cloneable canonical mutable state")
if "pub struct ProviderExecutionPermit" in provider_source:
    raise SystemExit("provider execution permits must remain internal to runtime execution methods")
if "pub fn begin_external_provider_call" in turn_source:
    raise SystemExit("raw provider-call permit issuance must not be public")
if "pub struct TurnCancellation" in cancellation_source or "pub fn cancellation(&self)" in turn_source:
    raise SystemExit("raw turn cancellation authority must remain internal to ActiveTurn::interrupt")
if "pub struct ProviderExecutionContext" in provider_source:
    raise SystemExit("provider execution security context must remain runtime-owned")
if "EffectiveAuthority" in provider_source:
    raise SystemExit("provider module must not accept caller-supplied effective authority")

# Provider generation callbacks must remain sealed buffers, never caller-defined transport hooks.
integration_source = (CRATES / "vpr-integration" / "src" / "lib.rs").read_text(encoding="utf-8")
for trait_name in ("GeneratedTextSink", "GeneratedAudioSink", "GeneratedVideoSink"):
    sealed_signature = f"pub trait {trait_name}: sealed::{trait_name}"
    if sealed_signature not in integration_source:
        raise SystemExit(f"{trait_name} must remain sealed against external transport implementations")
    for src_dir in CRATES.glob("*/src"):
        if src_dir.parent.name == "vpr-integration":
            continue
        for path in src_dir.rglob("*.rs"):
            if f"impl {trait_name} for" in path.read_text(encoding="utf-8"):
                raise SystemExit(
                    f"external {trait_name} implementation can bypass canonical delivery: {path.relative_to(ROOT)}"
                )

# Prevent production God Files from reappearing. Tests are allowed to be larger evidence bundles.
MAX_PRODUCTION_RUST_LINES = 600
MAX_RUNTIME_LIB_LINES = 120
for src_dir in CRATES.glob("*/src"):
    for path in src_dir.rglob("*.rs"):
        relative_source = path.relative_to(src_dir)
        if relative_source.name == "tests.rs" or relative_source.parts[0] == "tests":
            continue
        line_count = len(path.read_text(encoding="utf-8").splitlines())
        limit = (
            MAX_RUNTIME_LIB_LINES
            if path == runtime_src / "lib.rs"
            else MAX_PRODUCTION_RUST_LINES
        )
        if line_count > limit:
            raise SystemExit(
                f"God File guard: {path.relative_to(ROOT)} has {line_count} lines (limit {limit})"
            )

print("architecture-boundaries: PASS")
sys.exit(0)
