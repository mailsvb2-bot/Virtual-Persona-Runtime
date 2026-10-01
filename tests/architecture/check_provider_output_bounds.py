from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[2]
INTEGRATION = ROOT / "crates" / "vpr-integration" / "src"
HTTP = (INTEGRATION / "provider_http.rs").read_text(encoding="utf-8")
LIB = (INTEGRATION / "lib.rs").read_text(encoding="utf-8")
LLM = (INTEGRATION / "llm.rs").read_text(encoding="utf-8")

for required in (
    "MAX_PROVIDER_JSON_BODY_BYTES",
    "MAX_PROVIDER_BINARY_BODY_BYTES",
    "MAX_PROVIDER_STREAM_LINE_BYTES",
    "read_bounded_provider_body",
    "BoundedProviderLineReader",
):
    if required not in HTTP:
        raise SystemExit(f"canonical provider response bound drifted: {required}")

for required in (
    "MAX_GENERATED_AUDIO_BYTES",
    "MAX_GENERATED_AUDIO_DURATION_MILLIS",
    "MAX_GENERATED_VIDEO_BYTES",
    "MAX_GENERATED_VIDEO_FRAMES",
    "MAX_GENERATED_VIDEO_DURATION_MICROS",
):
    if required not in LIB:
        raise SystemExit(f"canonical generated media bound drifted: {required}")

if "MAX_GENERATED_TEXT_BYTES" not in LLM:
    raise SystemExit("canonical generated text bound drifted")

forbidden = {
    r"\bresponse\s*\.\s*json\s*\(\s*\)": "unbounded response.json()",
    r"\bresponse\s*\.\s*bytes\s*\(\s*\)": "unbounded response.bytes()",
    r"\bresponse\s*\.\s*text\s*\(\s*\)": "unbounded response.text()",
    r"\.read_to_end\s*\(": "unbounded read_to_end()",
    r"\.read_to_string\s*\(": "unbounded read_to_string()",
    r"BufReader::new\([^\n]*\)\.lines\s*\(": "unbounded BufRead::lines() framing",
}

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
    for pattern, description in forbidden.items():
        if re.search(pattern, sources):
            raise SystemExit(f"{crate_root.name} contains {description}: {pattern}")

streaming_crates = {
    "vpr-provider-openai-compatible",
    "vpr-provider-anthropic",
    "vpr-provider-gemini",
}
for crate_name in streaming_crates:
    source = "\n".join(
        path.read_text(encoding="utf-8")
        for path in sorted((ROOT / "crates" / crate_name / "src").rglob("*.rs"))
    )
    if "BoundedProviderLineReader" not in source:
        raise SystemExit(f"{crate_name} bypasses bounded provider stream framing")

json_body_crates = {
    "vpr-provider-openai-transcription",
    "vpr-provider-deepgram-stt",
    "vpr-provider-did-agent-streams",
    "vpr-provider-local-open-source",
}
binary_body_crates = {
    "vpr-provider-openai-speech",
    "vpr-provider-elevenlabs-tts",
}
for crate_name in json_body_crates | binary_body_crates:
    source = "\n".join(
        path.read_text(encoding="utf-8")
        for path in sorted((ROOT / "crates" / crate_name / "src").rglob("*.rs"))
    )
    if "read_bounded_provider_body" not in source:
        raise SystemExit(f"{crate_name} bypasses bounded provider body reads")

if not reqwest_provider_crates:
    raise SystemExit("expected at least one reqwest-backed provider adapter")

print("provider-output-bounds: PASS " + ",".join(reqwest_provider_crates))
