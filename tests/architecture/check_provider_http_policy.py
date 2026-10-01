from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[2]
POLICY = ROOT / "crates" / "vpr-integration" / "src" / "provider_http.rs"

policy = POLICY.read_text(encoding="utf-8")
for required in (
    "build_provider_http_client",
    ".redirect(reqwest::redirect::Policy::none())",
):
    if required not in policy:
        raise SystemExit(f"canonical provider HTTP policy drifted: {required}")

reqwest_provider_crates = []
for manifest in sorted((ROOT / "crates").glob("vpr-provider-*/Cargo.toml")):
    manifest_text = manifest.read_text(encoding="utf-8")
    if "reqwest" not in manifest_text:
        continue
    crate_root = manifest.parent
    sources = "\n".join(
        source.read_text(encoding="utf-8")
        for source in sorted((crate_root / "src").rglob("*.rs"))
    )
    reqwest_provider_crates.append(crate_root.name)
    if "build_provider_http_client" not in sources:
        raise SystemExit(
            f"{crate_root.name} uses reqwest without the canonical provider HTTP client policy"
        )
    for forbidden in (
        r"\bClient::builder\s*\(",
        r"\bClient::new\s*\(",
        r"reqwest::blocking::Client::builder\s*\(",
        r"reqwest::blocking::Client::new\s*\(",
    ):
        if re.search(forbidden, sources):
            raise SystemExit(
                f"{crate_root.name} bypasses the canonical provider HTTP client policy: {forbidden}"
            )

if not reqwest_provider_crates:
    raise SystemExit("expected at least one reqwest-backed provider adapter")

print("provider-http-policy: PASS " + ",".join(reqwest_provider_crates))
