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
    "vpr-provider-openai-speech": {"vpr-integration"},
    "vpr-provider-elevenlabs-tts": {"vpr-integration"},
    "vpr-provider-did-agent-streams": {"vpr-integration"},
    "vpr-evaluation": {"vpr-domain"},
    "vpr-live-proof": {"vpr-domain", "vpr-evaluation", "vpr-integration", "vpr-owner-lab", "vpr-policy", "vpr-runtime"},
    "vpr-owner-lab": {
        "vpr-domain",
        "vpr-integration",
        "vpr-policy",
        "vpr-provider-anthropic",
        "vpr-provider-deepgram-stt",
        "vpr-provider-did-agent-streams",
        "vpr-provider-gemini",
        "vpr-provider-openai-compatible",
        "vpr-provider-openai-transcription",
        "vpr-runtime",
    },
    "vpr-rt0-smoke": {
        "vpr-domain",
        "vpr-integration",
        "vpr-policy",
        "vpr-provider-anthropic",
        "vpr-provider-gemini",
        "vpr-provider-openai-compatible",
        "vpr-provider-openai-transcription",
        "vpr-provider-deepgram-stt",
        "vpr-provider-openai-speech",
        "vpr-provider-elevenlabs-tts",
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
turn_state_source = (runtime_src / "turn_state.rs").read_text(encoding="utf-8")
if "pub struct TurnMutableState" in turn_state_source:
    raise SystemExit("canonical mutable turn state must remain runtime-internal")
turn_state_derive = re.search(
    r"#\[derive\(([^)]*)\)\]\s*pub\(crate\) struct TurnMutableState\b",
    turn_state_source,
    re.MULTILINE,
)
if turn_state_derive is not None and any(
    item.strip() == "Clone" for item in turn_state_derive.group(1).split(",")
):
    raise SystemExit("TurnMutableState must remain non-cloneable canonical mutable state")
if "state: Arc<Mutex<TurnMutableState>>" not in turn_source:
    raise SystemExit("ActiveTurn canonical mutable state must remain one shared synchronized cell")
if "pub(crate) turn: Turn" in turn_source or "pub(crate) output_segments:" in turn_source:
    raise SystemExit("ActiveTurn must not split canonical mutable state across independent fields")
if "pub struct ProviderExecutionContext" in provider_source:
    raise SystemExit("provider execution security context must remain runtime-owned")
if "EffectiveAuthority" in provider_source:
    raise SystemExit("provider module must not accept caller-supplied effective authority")

delivery_source = (runtime_src / "delivery.rs").read_text(encoding="utf-8")
if "pub fn mark_output_sent" in turn_source or "pub fn mark_output_played" in turn_source:
    raise SystemExit("output delivery checkpoints must not be publicly mutable without transport evidence")
handle_match = re.search(r"pub struct OutputDeliveryHandle\s*\{([^}]*)\}", delivery_source, re.DOTALL)
if handle_match is None or re.search(r"\bpub\s+\w+\s*:", handle_match.group(1)):
    raise SystemExit("OutputDeliveryHandle fields must remain runtime-issued and non-forgeable")
handle_derive = re.search(
    r"#\[derive\(([^)]*)\)\]\s*pub struct OutputDeliveryHandle\b",
    delivery_source,
    re.MULTILINE,
)
if handle_derive is not None and any(
    item.strip() == "Clone" for item in handle_derive.group(1).split(",")
):
    raise SystemExit("OutputDeliveryHandle must remain non-cloneable capability evidence")
if "pub fn deliver_text" not in delivery_source or "RealtimeOutputPort" not in delivery_source:
    raise SystemExit("canonical text delivery must remain bound to RealtimeOutputPort")

media_timeline_source = (runtime_src / "media_timeline.rs").read_text(encoding="utf-8")
media_delivery_source = (runtime_src / "media_delivery.rs").read_text(encoding="utf-8")
session_source = (runtime_src / "session.rs").read_text(encoding="utf-8")
transport_source = (CRATES / "vpr-integration" / "src" / "transport.rs").read_text(encoding="utf-8")
if "media_timeline: MediaTimeline" not in session_source or "media_timeline: MediaTimeline" not in turn_source:
    raise SystemExit("canonical MediaTimeline must remain session-owned and shared with turns")
if "bound_epoch: u64" not in media_timeline_source:
    raise SystemExit("turn media mapping must remain bound to its creation epoch")
turn_media_match = re.search(
    r"#\[derive\(([^)]*)\)\]\s*pub\(crate\) struct TurnMediaState",
    media_timeline_source,
    re.MULTILINE,
)
if turn_media_match is None:
    raise SystemExit("TurnMediaState must remain runtime-internal with an explicit derive boundary")
if any(item.strip() == "Clone" for item in turn_media_match.group(1).split(",")):
    raise SystemExit("TurnMediaState must remain runtime-internal and non-cloneable")
for method in ("send_audio", "send_video", "flush_media"):
    if f"fn {method}" not in transport_source:
        raise SystemExit(f"RealtimeOutputPort must retain canonical media method {method}")
for method in ("deliver_audio", "deliver_video_frame", "interrupt_and_flush_media"):
    if f"pub fn {method}" not in media_delivery_source:
        raise SystemExit(f"runtime media contract missing {method}")
if "frame.timestamp_micros" not in media_delivery_source or "next_video_media_stamp" not in media_delivery_source:
    raise SystemExit("provider video timestamps must pass through runtime timeline normalization")

# Live-proof preflight must reuse Owner Lab provider composition and remain evidence-only.
live_proof_src = CRATES / "vpr-live-proof" / "src"
live_proof_text = "\n".join(path.read_text(encoding="utf-8") for path in live_proof_src.rglob("*.rs"))
for required in (
    "ProviderBundle::from_env(true)",
    "VPR_LIVE_PROOF_ALLOW_EGRESS",
    '"rev-parse", "HEAD"',
    '"status", "--porcelain", "--untracked-files=all"',
    "ProviderStateManifest",
    "configuration_fingerprint_sha256",
):
    if required not in live_proof_text:
        raise SystemExit(f"RT0 live-proof preflight missing mandatory invariant {required}")
for forbidden in ("VPR_DID_API_KEY", "VPR_OWNER_LAB_STT_API_KEY", "VPR_OWNER_LAB_LLM_API_KEY"):
    if forbidden in live_proof_text:
        raise SystemExit(f"live-proof layer must not read provider secrets directly: {forbidden}")


probe_source = (live_proof_src / "probe.rs").read_text(encoding="utf-8")
for required_probe_boundary in (
    "execute_stt(",
    "execute_llm(",
    "OwnerLabEngine::new",
    ".start(OwnerLabStartRequest { consent: true })",
    ".close()",
    "conversation_evidence: false",
    "output_delivery_proven: false",
    "input_audio_sha256",
    "input_audio_millis",
    "turn.begin_output()",
    "turn.complete()",
    "turn.fail()",
):
    if required_probe_boundary not in probe_source:
        raise SystemExit(
            f"RT0 live-provider probe missing canonical/evidence boundary {required_probe_boundary}"
        )
for forbidden_probe_call in (".transcribe(", ".stream(", ".create_session("):
    if forbidden_probe_call in probe_source:
        raise SystemExit(
            f"RT0 live-provider probe must not bypass canonical runtime with {forbidden_probe_call}"
        )
for forbidden_probe_payload in (
    "pub transcript:",
    "pub reply:",
    "pub sdp:",
    "provider_stream_id",
    "provider_session_id",
):
    if forbidden_probe_payload in probe_source:
        raise SystemExit(
            f"RT0 live-provider probe must not serialize sensitive/raw payload field {forbidden_probe_payload}"
        )

# Evaluation may inspect canonical domain evidence, but it must not become a runtime/provider brain.
evaluation_src = CRATES / "vpr-evaluation" / "src"
for path in evaluation_src.rglob("*.rs"):
    text = path.read_text(encoding="utf-8")
    for forbidden in (
        "vpr_runtime",
        "vpr_policy",
        "vpr_integration",
        "vpr_provider_",
    ):
        if forbidden in text:
            raise SystemExit(
                f"evaluation harness has forbidden runtime/provider dependency {forbidden!r} in {path.relative_to(ROOT)}"
            )

golden_source = (evaluation_src / "golden.rs").read_text(encoding="utf-8")
for required in (
    "FalseOwnerAttribution",
    "PrivateContextLeak",
    "CancelledOutputMarkedSpoken",
    "PersonaIdentityDrift",
    "VerifiedOwnerOpinion::try_from",
    "RT0_GOLDEN_SCHEMA",
):
    if required not in golden_source:
        raise SystemExit(f"RT0 Golden evaluator missing mandatory invariant {required}")
if '<evaluation-redacted>' not in golden_source:
    raise SystemExit("RT0 Golden evaluator must not copy owner claim text into attribution checks")
observation_derive = re.search(
    r"#\[derive\(([^)]*)\)\](?:\s*#\[[^\]]+\])*\s*pub struct GoldenObservation\b",
    golden_source,
    re.MULTILINE,
)
if observation_derive is None:
    raise SystemExit("RT0 Golden observation boundary is missing")
if any(item.strip() == "Serialize" for item in observation_derive.group(1).split(",")):
    raise SystemExit("raw Golden observations must not become serializable report material")

binding_source = (evaluation_src / "binding.rs").read_text(encoding="utf-8")
for required in (
    "RT0_EVIDENCE_BINDING_SCHEMA",
    "RT0_PROVIDER_STATE_SCHEMA",
    "CandidateShaMismatch",
    "SuiteDigestMismatch",
    "ReleaseSpecDigestMismatch",
    "ProviderStateDigestMismatch",
    "ProviderRole::Stt",
    "ProviderRole::Llm",
    "ProviderRole::Avatar",
    "configuration_fingerprint_sha256",
    "evidence_input_sha256",
    "deny_unknown_fields",
    "sha256_hex",
):
    if required not in binding_source:
        raise SystemExit(f"RT0 evidence binding missing fail-closed invariant {required}")

evaluation_main = (evaluation_src / "main.rs").read_text(encoding="utf-8")
for required in (
    "evaluate_bound_golden_suite",
    "release_spec_bytes",
    "provider_state_bytes",
    "candidate_sha",
    "report.golden.failed",
):
    if required not in evaluation_main:
        raise SystemExit(f"RT0 evaluation CLI missing bound-evidence contract {required}")


exit_source = (evaluation_src / "exit.rs").read_text(encoding="utf-8")
for required in (
    "RT0_EXIT_EVIDENCE_SCHEMA",
    "EvidenceOrigin::Real",
    "GoldenSetNotPassed",
    "OwnerConversationNotReal",
    "VisitorConversationNotReal",
    "AcceptanceMatrixIncomplete",
    "TextLatencyExceeded",
    "AudioLatencyExceeded",
    "InterruptionLatencyExceeded",
    "VideoLatencyExceeded",
    "AvSyncExceeded",
    "ReconnectLatencyExceeded",
    "CostNotMeasured",
    "PrivateContextLeakageAccepted",
    "FalseOwnerAttributionAccepted",
    "HumanEvaluationNotUsable",
    "KnownLimitationsNotReviewed",
    "ProviderStateMismatch",
    "provider_state_bytes",
    "live_provider_probe_sha256",
    "LiveProviderProbeDigestMismatch",
    "LiveProviderProbeCandidateMismatch",
    "LiveProviderProbeProviderStateMismatch",
    "1_000",
    "2_500",
    "1_500",
    "3_000",
    "500",
    "120",
    "5_000",
    "deny_unknown_fields",
):
    if required not in exit_source:
        raise SystemExit(f"RT0 exit evidence evaluator missing mandatory invariant {required}")
if len(exit_source.splitlines()) > 600:
    raise SystemExit("RT0 exit evidence evaluator became a God File (>600 lines)")

exit_cli = (evaluation_src / "bin" / "rt0_exit_gate.rs").read_text(encoding="utf-8")
for required in (
    "evaluate_rt0_exit_evidence",
    "golden_report_bytes",
    "provider_state_bytes",
    "ProviderStateManifest",
    "GoldenEvidenceBundle",
    "golden_evidence_bytes",
    "live_provider_probe_bytes",
    "LiveProviderProbeReceipt",
    "release_spec_bytes",
    "exact_candidate_sha",
    "report.ready",
):
    if required not in exit_cli:
        raise SystemExit(f"RT0 exit evidence CLI missing exact-candidate contract {required}")

# Realtime-avatar signaling must remain provider-neutral, secret-safe and outside runtime.
avatar_source = (CRATES / "vpr-integration" / "src" / "avatar.rs").read_text(encoding="utf-8")
if "pub trait RealtimeAvatarPort" not in avatar_source:
    raise SystemExit("provider-neutral RealtimeAvatarPort must remain in vpr-integration")
for method in ("create_session", "submit_answer", "submit_ice_candidate", "speak_text", "speak_audio_url", "close_session"):
    if f"fn {method}" not in avatar_source:
        raise SystemExit(f"RealtimeAvatarPort missing lifecycle operation {method}")
for secret_type in ("WebRtcSessionDescription", "WebRtcIceServer", "WebRtcIceCandidate", "RealtimeAvatarSession"):
    derive = re.search(
        rf"#\[derive\(([^)]*)\)\]\s*pub struct {secret_type}\b",
        avatar_source,
        re.MULTILINE,
    )
    if derive is not None and any(item.strip() == "Debug" for item in derive.group(1).split(",")):
        raise SystemExit(f"{secret_type} must not derive raw Debug over WebRTC secrets")
    if f"impl Debug for {secret_type}" not in avatar_source:
        raise SystemExit(f"{secret_type} must retain redacted Debug implementation")

did_source = (CRATES / "vpr-provider-did-agent-streams" / "src" / "lib.rs").read_text(encoding="utf-8")
did_config_derive = re.search(
    r"#\[derive\(([^)]*)\)\]\s*pub struct DidAgentStreamsConfig\b",
    did_source,
    re.MULTILINE,
)
if did_config_derive is not None and any(
    item.strip() in {"Debug", "Clone"} for item in did_config_derive.group(1).split(",")
):
    raise SystemExit("D-ID config must not expose or clone API credentials through derived traits")
if "impl RealtimeAvatarPort for DidAgentStreamsAvatar" not in did_source:
    raise SystemExit("D-ID adapter must remain behind RealtimeAvatarPort")

owner_lab_src = CRATES / "vpr-owner-lab" / "src"
owner_lab_main = (owner_lab_src / "main.rs").read_text(encoding="utf-8")
owner_lab_state = (owner_lab_src / "state.rs").read_text(encoding="utf-8")
owner_lab_providers = (owner_lab_src / "providers.rs").read_text(encoding="utf-8")
owner_lab_ui_root = CRATES / "vpr-owner-lab" / "ui"
owner_lab_bundle = owner_lab_ui_root / "dist" / "app.js"
if not owner_lab_bundle.is_file():
    raise SystemExit("Owner Lab browser bundle must remain versioned for Rust include_str embedding")
owner_lab_ui_files = [
    path
    for path in owner_lab_ui_root.rglob("*")
    if path.is_file()
    and "node_modules" not in path.parts
    and path.suffix in {".ts", ".js", ".html", ".css", ".json"}
]
owner_lab_ui = "\n".join(path.read_text(encoding="utf-8") for path in owner_lab_ui_files)
if 'format!("127.0.0.1:{port}")' not in owner_lab_main:
    raise SystemExit("Owner Lab HTTP listener must remain loopback-only")
for required in (
    "X-VPR-CSRF",
    "VPR_OWNER_LAB_ALLOW_EGRESS",
    "valid_host",
    "valid_origin",
    "Content-Security-Policy",
):
    if required not in owner_lab_main:
        raise SystemExit(f"Owner Lab backend missing security boundary {required}")
for required_provider_boundary in (
    "VPR_DID_API_KEY",
    "VPR_OWNER_LAB_STT_API_KEY",
    "VPR_OWNER_LAB_LLM_API_KEY",
    "ProviderBundle",
    "configuration_fingerprint_sha256",
):
    if required_provider_boundary not in owner_lab_providers:
        raise SystemExit(
            f"Owner Lab shared provider composition missing boundary {required_provider_boundary}"
        )
if "open_realtime_avatar" not in owner_lab_state or ".create_session(" in owner_lab_state:
    raise SystemExit("Owner Lab must use canonical runtime avatar binding, not provider session creation")
if "if !self.egress_enabled" not in owner_lab_state or "if !request.consent" not in owner_lab_state:
    raise SystemExit("Owner Lab production start path must retain process egress and explicit-consent gates")
for forbidden in ("VPR_DID_API_KEY", "api.d-id.com", "integration-secret", "secret-key"):
    if forbidden in owner_lab_ui:
        raise SystemExit(f"Owner Lab UI must not contain provider secrets/endpoints: {forbidden}")
if 'fetch("http' in owner_lab_ui or "fetch('http" in owner_lab_ui:
    raise SystemExit("Owner Lab UI must use same-origin backend APIs only")
for required_ui_recovery in (
    "backendSessionPresent",
    "syncStatus",
    "/api/session/close",
    'window.addEventListener("pagehide"',
    "keepalive: true",
):
    if required_ui_recovery not in owner_lab_ui:
        raise SystemExit(f"Owner Lab UI missing cleanup/recovery contract: {required_ui_recovery}")

owner_lab_voice = (owner_lab_src / "state" / "voice.rs").read_text(encoding="utf-8")
owner_lab_voice_providers = owner_lab_providers
owner_lab_mic_worklet = owner_lab_ui_root / "mic-worklet.js"
if not owner_lab_mic_worklet.is_file():
    raise SystemExit("Owner Lab push-to-talk must retain a versioned AudioWorklet processor")
for required_voice_runtime in (
    "execute_stt",
    "execute_llm",
    "speak_realtime_avatar_text",
    "interrupt_handle",
):
    if required_voice_runtime not in owner_lab_voice:
        raise SystemExit(f"Owner Lab voice path must remain canonical: {required_voice_runtime}")
for required_voice_http in (
    "/api/voice/turn",
    "application/octet-stream",
    "active_voice_interrupt",
    "voice_busy",
    "voice_cancel_requested",
    "session_end_requested",
    "request_voice_cancel",
    "compare_exchange",
):
    if required_voice_http not in owner_lab_main:
        raise SystemExit(f"Owner Lab voice HTTP boundary missing {required_voice_http}")
for provider_name in (
    "openai-transcription",
    "deepgram",
    "openai-compatible",
    "anthropic",
    "gemini",
):
    if provider_name not in owner_lab_voice_providers:
        raise SystemExit(f"Owner Lab voice provider selection missing {provider_name}")
for forbidden_browser_secret in (
    "VPR_OWNER_LAB_STT_API_KEY",
    "VPR_OWNER_LAB_LLM_API_KEY",
    "VPR_OWNER_LAB_STT_ENDPOINT",
    "VPR_OWNER_LAB_LLM_ENDPOINT",
):
    if forbidden_browser_secret in owner_lab_ui:
        raise SystemExit(f"Owner Lab browser must not own provider configuration: {forbidden_browser_secret}")
if "AudioWorkletNode" not in owner_lab_ui or "apiBinary" not in owner_lab_ui:
    raise SystemExit("Owner Lab voice UI must use AudioWorklet plus binary same-origin upload")
if "MAX_VOICE_SAMPLES" not in owner_lab_ui or ".subarray(0, MAX_VOICE_SAMPLES)" not in owner_lab_ui:
    raise SystemExit("Owner Lab voice UI must cap actual PCM samples before upload")
if "ScriptProcessor" in owner_lab_ui or "MediaRecorder" in owner_lab_ui:
    raise SystemExit("Owner Lab voice capture must not regress to deprecated/encoded browser capture")

owner_lab_evidence = (owner_lab_src / "evidence.rs").read_text(encoding="utf-8")
for required_evidence_boundary in (
    "rt0-owner-lab-session-evidence-0.1",
    "LabSessionEvidenceRecorder",
    "LabMediaEvidenceInput",
    "stt_millis",
    "llm_millis",
    "server_total_millis",
):
    if required_evidence_boundary not in owner_lab_evidence:
        raise SystemExit(f"Owner Lab session evidence missing boundary {required_evidence_boundary}")
for forbidden_evidence_payload in (
    "transcript: String",
    "reply: String",
    "pcm:",
    "sdp:",
):
    if forbidden_evidence_payload in owner_lab_evidence:
        raise SystemExit(f"Owner Lab session evidence must not persist raw payload: {forbidden_evidence_payload}")
owner_lab_http_evidence = (owner_lab_src / "http_evidence.rs").read_text(encoding="utf-8")
for required_media_endpoint in ("/api/evidence/media", "/api/evidence/session"):
    if required_media_endpoint not in owner_lab_main:
        raise SystemExit(f"Owner Lab HTTP evidence endpoint missing {required_media_endpoint}")
if "X-VPR-Evidence-Request" not in owner_lab_http_evidence:
    raise SystemExit("Owner Lab HTTP evidence correlation header must remain isolated in http_evidence.rs")
for required_browser_media_evidence in (
    "requestVideoFrameCallback",
    "AnalyserNode",
    "/api/evidence/media",
    "X-VPR-Evidence-Request",
):
    if required_browser_media_evidence not in owner_lab_ui:
        raise SystemExit(f"Owner Lab browser media evidence missing {required_browser_media_evidence}")
for forbidden_owner_lab_delivery_claim in ("mark_output_played", "acknowledge_output_played"):
    if forbidden_owner_lab_delivery_claim in owner_lab_main or forbidden_owner_lab_delivery_claim in owner_lab_evidence:
        raise SystemExit(
            f"Owner Lab browser evidence must not self-promote to canonical played state: {forbidden_owner_lab_delivery_claim}"
        )

turn_interrupt_match = re.search(
    r"pub struct TurnInterruptHandle\s*\{([^}]*)\}", turn_source, re.DOTALL
)
if turn_interrupt_match is None or re.search(r"\bpub\s+\w+\s*:", turn_interrupt_match.group(1)):
    raise SystemExit("TurnInterruptHandle must remain opaque and runtime-issued")
if "pub fn interrupt_handle" not in turn_source or "pub fn interrupt(&self)" not in turn_source:
    raise SystemExit("runtime must retain narrow concurrent turn interruption capability")

avatar_runtime_source = (runtime_src / "avatar_runtime.rs").read_text(encoding="utf-8")
handle_match = re.search(r"pub struct RealtimeAvatarHandle\s*\{([^}]*)\}", avatar_runtime_source, re.DOTALL)
if handle_match is None or re.search(r"\bpub\s+\w+\s*:", handle_match.group(1)):
    raise SystemExit("RealtimeAvatarHandle fields must remain runtime-issued and non-forgeable")
handle_derive = re.search(
    r"#\[derive\(([^)]*)\)\]\s*pub struct RealtimeAvatarHandle\b",
    avatar_runtime_source,
    re.MULTILINE,
)
if handle_derive is not None and any(
    item.strip() == "Clone" for item in handle_derive.group(1).split(",")
):
    raise SystemExit("RealtimeAvatarHandle must remain non-cloneable session capability evidence")
for method in (
    "open_realtime_avatar",
    "submit_realtime_avatar_answer",
    "submit_realtime_avatar_ice",
    "speak_realtime_avatar_text",
    "speak_realtime_avatar_audio_url",
    "interrupt_realtime_avatar",
    "close_realtime_avatar",
):
    if f"pub fn {method}" not in avatar_runtime_source:
        raise SystemExit(f"runtime realtime-avatar contract missing {method}")
if "session_id: SessionId" not in turn_source:
    raise SystemExit("ActiveTurn must remain bound to its canonical session for avatar handle validation")
close_match = re.search(
    r"fn close_session\(\s*&self,\s*session: &RealtimeAvatarSession\s*\)",
    avatar_source,
    re.MULTILINE,
)
if close_match is None:
    raise SystemExit("avatar cleanup must remain independent of cancelled turn work")

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
