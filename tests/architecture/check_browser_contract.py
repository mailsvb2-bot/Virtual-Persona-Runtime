from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
UI = ROOT / "crates" / "vpr-owner-lab" / "ui"
INDEX = UI / "index.html"
E2E = UI / "e2e" / "owner-journey.spec.ts"
BACKEND_E2E = UI / "e2e" / "backend-owner-journey.spec.ts"
BACKEND_CONFIG = UI / "playwright.backend.config.ts"
BACKEND_PROVIDER = UI / "e2e" / "backend-provider.mjs"
BACKEND_LAUNCHER = ROOT / "tests" / "e2e" / "run_owner_lab_backend.py"
VOICE_E2E = UI / "e2e" / "backend-voice-journey.spec.ts"
VOICE_MEDIA_FIXTURE = UI / "e2e" / "fake-webrtc-media-runtime.js"
VOICE_JOURNEY_DRIVER = UI / "e2e" / "voice-journey-driver.js"
VOICE_JOURNEY_CONTRACT = UI / "e2e" / "voice-journey-contract.ts"
PROVIDER_BOOTSTRAP = UI / "e2e" / "provider-bootstrap.ts"
EXPRESSIVE_E2E = UI / "e2e" / "backend-expressive-journey.spec.ts"
EXPRESSIVE_JOURNEY_DRIVER = UI / "e2e" / "expressive-journey-driver.js"
EXPRESSIVE_CONFIG = UI / "playwright.expressive.config.ts"
EXPRESSIVE_LAUNCHER = ROOT / "tests" / "e2e" / "run_owner_lab_expressive_backend.py"
APP = UI / "src" / "app.ts"
BOOTSTRAP_CONTEXT = UI / "src" / "bootstrap-context.ts"
MEDIA_RUNTIME = UI / "src" / "media-runtime.ts"
SESSION_RUNTIME_STATE = UI / "src" / "session-runtime-state.ts"
VOICE_SCHEDULER = UI / "src" / "voice-command-scheduler.ts"
STYLES = UI / "styles.css"
FIXTURE_SERVER = UI / "e2e" / "server.mjs"
EVIDENCE_EXPORT = UI / "src" / "evidence-export.ts"
VOICE_CONFIG = UI / "playwright.voice.config.ts"
VOICE_PROVIDER = UI / "e2e" / "voice-provider-fixture.mjs"
VOICE_LAUNCHER = ROOT / "tests" / "e2e" / "run_owner_lab_voice_backend.py"
CI = ROOT / ".github" / "workflows" / "ci.yml"
SESSION_EVIDENCE = ROOT / "crates" / "vpr-evaluation" / "src" / "session_evidence.rs"
RT0_RELEASE_SPEC = ROOT / "docs" / "releases" / "RT0_RELEASE_SPEC.md"
HTTP_CLIENT_CONTROL = ROOT / "crates" / "vpr-owner-lab" / "src" / "http_client_control.rs"

index_html = INDEX.read_text(encoding="utf-8")
browser_e2e = E2E.read_text(encoding="utf-8")
backend_e2e = BACKEND_E2E.read_text(encoding="utf-8")
backend_config = BACKEND_CONFIG.read_text(encoding="utf-8")
backend_provider = BACKEND_PROVIDER.read_text(encoding="utf-8")
backend_launcher = BACKEND_LAUNCHER.read_text(encoding="utf-8")
voice_e2e = VOICE_E2E.read_text(encoding="utf-8")
voice_media_fixture = VOICE_MEDIA_FIXTURE.read_text(encoding="utf-8")
voice_journey_driver = VOICE_JOURNEY_DRIVER.read_text(encoding="utf-8")
voice_journey_contract = VOICE_JOURNEY_CONTRACT.read_text(encoding="utf-8")
provider_bootstrap = PROVIDER_BOOTSTRAP.read_text(encoding="utf-8")
expressive_e2e = EXPRESSIVE_E2E.read_text(encoding="utf-8")
expressive_journey_driver = EXPRESSIVE_JOURNEY_DRIVER.read_text(encoding="utf-8")
fake_livekit = (UI / "e2e" / "fake-livekit-client.js").read_text(encoding="utf-8")
expressive_config = EXPRESSIVE_CONFIG.read_text(encoding="utf-8")
expressive_launcher = EXPRESSIVE_LAUNCHER.read_text(encoding="utf-8")
app = APP.read_text(encoding="utf-8")
bootstrap_context = BOOTSTRAP_CONTEXT.read_text(encoding="utf-8")
media_runtime = MEDIA_RUNTIME.read_text(encoding="utf-8")
session_runtime_state = SESSION_RUNTIME_STATE.read_text(encoding="utf-8")
voice_scheduler = VOICE_SCHEDULER.read_text(encoding="utf-8")
styles = STYLES.read_text(encoding="utf-8")
fixture_server = FIXTURE_SERVER.read_text(encoding="utf-8")
evidence_export = EVIDENCE_EXPORT.read_text(encoding="utf-8")
voice_config = VOICE_CONFIG.read_text(encoding="utf-8")
voice_provider = VOICE_PROVIDER.read_text(encoding="utf-8")
voice_launcher = VOICE_LAUNCHER.read_text(encoding="utf-8")
default_config = (UI / "playwright.config.ts").read_text(encoding="utf-8")
ci = CI.read_text(encoding="utf-8")
session_evidence = SESSION_EVIDENCE.read_text(encoding="utf-8")
rt0_release_spec = RT0_RELEASE_SPEC.read_text(encoding="utf-8")
http_client_control = HTTP_CLIENT_CONTROL.read_text(encoding="utf-8")

# Test doubles may replace browser media primitives only at the composition seam.
# They must never participate in production HTTP completion ordering or mutate the
# runtime after navigation. These are architecture invariants, not timing heuristics.
for forbidden in (
    "afterApiResponse",
    "notifyTestApiResponse",
    "__vprTestMediaRuntime",
):
    if forbidden in app or forbidden in media_runtime:
        raise SystemExit(f"Owner Lab production runtime contains forbidden test orchestration: {forbidden}")
for required in (
    "export type MediaRuntimeOverrides",
    "window.__vprMediaRuntime",
    "runtimeMediaDevices",
    "createRuntimePeerConnection",
    "setMediaSrcObject",
):
    if required not in media_runtime:
        raise SystemExit(f"Owner Lab media adapter missing dependency seam: {required}")
if "__vprMediaRuntime" in app:
    raise SystemExit("Owner Lab application must consume the media adapter, not the global override directly")
if 'from "./media-runtime.js"' not in app:
    raise SystemExit("Owner Lab application must compose media through the extracted adapter")

for forbidden in ("let backendStatus", "let realtimeReadiness", "let providerPlaybackId"):
    if forbidden in app:
        raise SystemExit(f"Owner Lab contains split mutable session ownership: {forbidden}")
for required in (
    "export class SessionRuntimeState",
    "applyBackend(status: LabStatus)",
    "patchBackend(patch: Partial<LabStatus>)",
    "resetRealtime(): void",
):
    if required not in session_runtime_state:
        raise SystemExit(f"Owner Lab centralized session runtime state missing: {required}")
if 'from "./session-runtime-state.js"' not in app:
    raise SystemExit("Owner Lab application must compose the extracted session runtime state")

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
    "/api/avatar/answer",
    "INVALID_STATE_TRANSITION",
    "revoked-must-not-egress",
    "providerAfterRevokedJson.requests",
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

for required in (
    "backend-owner-journey.spec.ts",
    "backend-voice-journey.spec.ts",
    "backend-expressive-journey.spec.ts",
):
    if required not in default_config:
        raise SystemExit(f"Default browser contract must exclude {required}")

for required in ("VPR_DID_ENDPOINT", "VPR_DID_API_KEY", "cargo", "vpr-owner-lab"):
    if required not in backend_launcher:
        raise SystemExit(f"Owner Lab backend launcher missing: {required}")


# Voice browser proof is intentionally split across three layers:
# - the Playwright controller owns setup/navigation and external HTTP assertions only;
# - the in-page driver owns real DOM interactions after navigation;
# - the contract module owns evidence/provider assertions.
for required in (
    'page.addInitScript({ path: "e2e/voice-journey-driver.js" })',
    'page.route("**/__browser-journey"',
    "expect.poll",
    "assertBrowserJourneyEvidence",
    "assertProviderRequests",
    'await page.goto("/");',
):
    if required not in voice_e2e:
        raise SystemExit(f"Owner Lab Voice controller missing same-origin evidence contract: {required}")

for required in (
    "Текстовый вопрос владельца",
    "Спровоцируй отказ провайдера",
    "Восстановление после отказа",
    "Текстовый вопрос visitor",
    "__vprSetPeerConnectionState",
    "interruption_stopped",
    "reconnect_restored",
    "/api/evidence/session",
    'const reportUrl = "/__browser-journey";',
):
    if required not in voice_journey_driver:
        raise SystemExit(f"Owner Lab Voice in-page driver missing canonical journey step: {required}")
for required in ("Привет из браузера", "Что думает владелец?"):
    if required not in voice_provider:
        raise SystemExit(f"Owner Lab Voice STT fixture missing canonical transcript: {required}")

if "/__browser-journey" in voice_provider or "browserJourney" in voice_provider:
    raise SystemExit("Owner Lab provider fixture must not own browser journey control state")
if "http://127.0.0.1:18790" in voice_journey_driver:
    raise SystemExit("Owner Lab Voice driver must not depend on cross-origin provider control-plane calls")
if "waitForBrowserJourney" in voice_journey_contract:
    raise SystemExit("Owner Lab Voice contract must not retain the removed cross-origin polling path")

for required in (
    "voice_attempts",
    "canonical_playback_proven",
    "av_sync_proven",
    "web_rtc_estimated_playout_timestamp",
    "absolute_offset_millis",
    "PROVIDER_UNAVAILABLE",
    "Visitor permissions do not expose owner-reviewed personal context",
    "Bearer voice-stt-e2e-secret",
    "Bearer voice-llm-e2e-secret",
    "Basic voice-avatar-e2e-secret",
    "interruption_stopped",
    "reconnect_restored",
):
    if required not in voice_journey_contract:
        raise SystemExit(f"Owner Lab Voice evidence contract missing proof: {required}")

expressive_proof = expressive_e2e + "\n" + expressive_journey_driver
for required in (
    "LiveKit согласован",
    "canonical_playback_proven",
    "av_sync_proven",
    "did.speak",
    "did.interrupt",
    "__vprExpressiveDisconnect",
    "/v2/agents/voice-e2e-expressive-agent/sessions",
    "metric-stt",
    "metric-llm-first",
    "metric-av-sync",
    "metric-playback",
    "metric-cost",
    '"/v1/listen"',
    '"model=nova-3"',
    '"language=ru"',
    '"reasoning_effort":"none"',
    '"max_tokens":96',
    '"model":"deepseek-flash"',
):
    if required not in expressive_proof:
        raise SystemExit(f"Owner Lab Expressive browser proof missing: {required}")


def require_pre_navigation_media_runtime(source: str, installer: str, fixture_path: str, label: str) -> None:
    declaration = (
        f"const {installer} = async (page: Page): Promise<void> => {{\n"
        f'  await page.addInitScript({{ path: "{fixture_path}" }});'
    )
    if declaration not in source:
        raise SystemExit(
            f"{label} media runtime must be loaded as an isolated init-script fixture"
        )
    call = f"await {installer}(page);"
    goto = 'await page.goto("/");'
    call_index = source.find(call)
    goto_index = source.find(goto)
    if call_index < 0 or goto_index < 0 or call_index > goto_index:
        raise SystemExit(f"{label} media runtime must be installed before page.goto")


voice_goto_index = voice_e2e.find('await page.goto("/");')
for required in (
    'await page.addInitScript({ path: "e2e/fake-webrtc-media-runtime.js" });',
    "await installProviderAutoConnect(page);",
    'await page.addInitScript({ path: "e2e/voice-journey-driver.js" });',
):
    required_index = voice_e2e.find(required)
    if required_index < 0 or voice_goto_index < 0 or required_index > voice_goto_index:
        raise SystemExit(f"Owner Lab Voice pre-navigation composition missing or late: {required}")

for forbidden in (
    "prepareExpressiveRuntimeFakes",
    "prepareExpressiveVoiceCaptureFakes",
):
    if forbidden in expressive_e2e:
        raise SystemExit(
            f"Owner Lab Expressive runtime must be owned by the fake LiveKit SDK boundary, not {forbidden}"
        )
if 'await page.addInitScript({ path: "e2e/fake-livekit-client.js" });' not in expressive_e2e:
    raise SystemExit("Owner Lab Expressive E2E must load the fake SDK before navigation without network routing")
if 'await page.addInitScript({ path: "e2e/expressive-journey-driver.js" });' not in expressive_e2e:
    raise SystemExit("Owner Lab Expressive E2E must install its in-page journey driver before navigation")
for required in (
    'const reportUrl = "/__expressive_journey_report";',
    'postPhase("driver-started")',
    'postPhase("connected")',
    'postPhase("interrupt-complete")',
    'postPhase("disconnect-complete")',
):
    if required not in expressive_journey_driver:
        raise SystemExit(f"Owner Lab Expressive in-page journey driver missing lifecycle proof: {required}")
if 'page.route("**/__expressive_journey_report"' not in expressive_e2e:
    raise SystemExit("Owner Lab Expressive E2E missing same-origin terminal-report mailbox")
if "cdn.jsdelivr.net/npm/livekit-client" in expressive_e2e:
    raise SystemExit("Owner Lab Expressive E2E must not route the LiveKit CDN through Playwright")
for required in (
    "const installMediaRuntime = () => {",
    "installMediaRuntime();",
    "window.__vprMediaRuntime",
    "window.LivekitClient",
    "FakeAudioWorkletNode",
    "async getUserMedia()",
    "if (!testDevicesReady) return []",
):
    if required not in fake_livekit:
        raise SystemExit(f"Owner Lab fake LiveKit SDK missing deterministic media runtime: {required}")
if "navigator.mediaDevices" in fake_livekit:
    raise SystemExit("Owner Lab fake LiveKit SDK must never touch native browser media devices")
if "async connect() {\n      installMediaRuntime();" in fake_livekit:
    raise SystemExit("Owner Lab Expressive media runtime must be installed once before navigation")
if fake_livekit.find("installMediaRuntime();") > fake_livekit.find("window.LivekitClient"):
    raise SystemExit("Owner Lab Expressive media runtime must exist before the fake SDK is published")


for required in (
    "run_owner_lab_expressive_backend.py",
    "backend-expressive-journey.spec.ts",
):
    if required not in expressive_config:
        raise SystemExit(f"Owner Lab Expressive browser config missing: {required}")

for required in (
    "VPR_DID_AGENT_ID",
    "voice-e2e-expressive-agent",
    '"VPR_OWNER_LAB_STT_PROVIDER": "deepgram"',
    '"VPR_OWNER_LAB_STT_MODEL": "nova-3"',
    '"VPR_OWNER_LAB_LLM_PROVIDER": "deepseek"',
    '"VPR_OWNER_LAB_LLM_MODEL": "deepseek-flash"',
):
    if required not in expressive_launcher:
        raise SystemExit(f"Owner Lab Expressive launcher missing: {required}")

for required in (
    '"/api/avatar/client-interrupt"',
    "active_voice_interrupt.lock().clone()",
    "handle.interrupt()",
):
    if required not in http_client_control:
        raise SystemExit(f"Owner Lab browser interruption must cancel the active canonical voice turn: {required}")

for required in (
    'id="microphone-device"',
    "Системный микрофон по умолчанию",
    'id="microphone-level"',
    'id="microphone-level-text"',
):
    if required not in index_html:
        raise SystemExit(f"Owner Lab microphone selection DOM missing: {required}")

for required in (
    "enumerateDevices",
    "MICROPHONE_STORAGE_KEY",
    'deviceId: { exact: selectedDeviceId }',
    "getAudioTracks",
    '"devicechange"',
    "microphoneLevel.value",
    "Сигнал почти нулевой",
    "RMS",
):
    if required not in app:
        raise SystemExit(f"Owner Lab microphone device selection missing: {required}")

for required in (
    "__vprRequestedMicrophones",
    "Микрофон гарнитуры",
    'localStorage.setItem("vpr.owner-lab.microphone-device-id", "headset-mic")',
):
    if required not in voice_media_fixture:
        raise SystemExit(f"Owner Lab microphone fixture contract missing: {required}")
if "__vprRequestedMicrophones" not in voice_journey_driver:
    raise SystemExit("Owner Lab Voice driver must capture the microphone request evidence")
if 'expect(journey.requestedMicrophones).toContain("headset-mic")' not in voice_journey_contract:
    raise SystemExit("Owner Lab Voice contract must assert the persisted microphone preference was requested")

for required in (
    "estimatedPlayoutTimestamp",
    "/api/evidence/av-sync",
    "AV_SYNC_SAMPLE_COUNT",
    "AV_SYNC_MAX_ATTEMPTS",
    "getRTCStatsReport",
    "liveKitAudioTrack",
    "liveKitVideoTrack",
    "TrackUnsubscribed",
):
    if required not in app:
        raise SystemExit(f"Owner Lab A/V sync browser evidence missing: {required}")

for required in (
    "handleUnexpectedLiveKitDisconnect",
    '"/api/session/close"',
    "clearRealtimeMedia",
):
    if required not in app:
        raise SystemExit(f"Owner Lab LiveKit disconnect recovery missing: {required}")

if "sessionState.realtime.control" not in app:
    raise SystemExit("Owner Lab transport readiness must read the session-state control projection")
for required in (
    "control: boolean;",
    "audio: boolean;",
    "video: boolean;",
    "setRealtimeReadiness",
):
    if required not in session_runtime_state:
        raise SystemExit(f"Owner Lab modality readiness split missing from state owner: {required}")
for forbidden in (
    "sessionState.realtime.control =",
    "sessionState.realtime.audio =",
    "sessionState.realtime.video =",
    "sessionState.playbackId =",
):
    if forbidden in app:
        raise SystemExit(f"Owner Lab session state must not be mutated outside SessionRuntimeState: {forbidden}")
for required in (
    "setRealtimeReadiness",
    "setPlaybackId",
    "private backendValue",
    "private realtimeValue",
    "private playbackIdValue",
):
    if required not in session_runtime_state:
        raise SystemExit(f"Owner Lab session state owner missing canonical transition API: {required}")

for forbidden in (
    'sessionState.patchBackend({\n      session_state: "active"',
    'sessionState.patchBackend({\n            session_state: "active"',
):
    if forbidden in app:
        raise SystemExit("Owner Lab connect must not manufacture authoritative backend session state")
for required in (
    "let backendSessionStarted = false;",
    "backendSessionStarted = true;",
    "const startedStatus = await syncStatus();",
    'throw new Error("SESSION_START_STATE_MISMATCH")',
    "if (backendSessionStarted || backendSessionPresent())",
):
    if required not in app:
        raise SystemExit(f"Owner Lab authoritative connect lifecycle missing: {required}")
if "connect closes a started backend session when authoritative start status mismatches" not in browser_e2e:
    raise SystemExit("Owner Lab browser proof missing authoritative start cleanup regression coverage")
if "realtimeTransportReady" in app:
    raise SystemExit("Owner Lab must not collapse control/audio/video readiness into one flag")
if 'const voiceReady = sessionState.backend.conversation_readiness === "text_and_voice";' not in app:
    raise SystemExit("Owner Lab microphone readiness must follow canonical voice-provider readiness")
if 'const voiceReady = sessionState.backend.conversation_readiness === "text_and_voice"\n    && sessionState.realtime.audio' in app:
    raise SystemExit("Owner Lab microphone input must not depend on the avatar output-audio track")
if "await onSegment(event.segment)" in app or "onSegment(event.segment)" not in app:
    raise SystemExit("Owner Lab voice event ingestion must remain decoupled from playback backpressure")
for required in (
    "private queueTail: Promise<void> = Promise.resolve()",
    "this.queueTail.then(run, run)",
    "this.queueTail = scheduled.then(",
):
    if required not in voice_scheduler:
        raise SystemExit(f"Owner Lab playback scheduler must preserve strict FIFO serialization: {required}")

for required in (
    "#avatar",
    "width:100% !important",
    "max-width:100% !important",
    "object-fit:contain",
    ".shell > *",
    "min-width:0",
    "overflow-x:hidden",
):
    if required not in styles:
        raise SystemExit(f"Owner Lab realtime media containment missing: {required}")


for required in (
    'id="metric-stt"',
    'id="metric-llm"',
    'id="metric-llm-first"',
    'id="metric-server-total"',
    'id="metric-text-first"',
    'id="metric-first-audio"',
    'id="metric-video-ready"',
    'id="metric-av-sync"',
    'id="metric-playback"',
    'id="metric-cost"',
):
    if required not in index_html:
        raise SystemExit(f"Owner Lab readable telemetry DOM missing: {required}")

for required in (
    "renderTelemetry",
    "isSessionEvidenceSnapshot",
    "llm_first_meaningful_millis",
    "estimated_cost_microunits",
    "provider_charge_microunits",
    "провайдер не сообщил стоимость",
):
    if required not in app:
        raise SystemExit(f"Owner Lab readable telemetry rendering missing: {required}")

for required in ('id="export-evidence"', "Скачать evidence snapshot"):
    if required not in index_html:
        raise SystemExit(f"Owner Lab evidence export DOM missing: {required}")

if '["/evidence-export.js", "dist/evidence-export.js"]' not in fixture_server:
    raise SystemExit("Owner Lab browser fixture must serve generated evidence export module")

if '<script type="module" src="/evidence-export.js"></script>' not in index_html:
    raise SystemExit("Owner Lab evidence export module must load independently from the main app")

for required in (
    "/api/bootstrap",
    "/api/evidence/session/export",
    "response.arrayBuffer()",
    'document.getElementById("export-evidence")',
    "session-${identity.session_sequence}-${identity.participant_role}.json",
):
    if required not in evidence_export:
        raise SystemExit(f"Owner Lab exact-byte evidence export missing: {required}")

for required in ("session-1-owner.json", "session-2-visitor.json"):
    if required not in backend_e2e:
        raise SystemExit(f"Owner Lab owner/visitor evidence export browser proof missing: {required}")

for source, required in (
    (app, "const AV_SYNC_SAMPLE_COUNT = 3;"),
    (session_evidence, "pub const RT0_AV_SYNC_SAMPLES_PER_REQUEST: u32 = 3;"),
    (rt0_release_spec, "Exactly three samples, sequence-numbered `1..=3`, are required"),
):
    if required not in source:
        raise SystemExit(f"Owner Lab A/V sync sample-count contract missing: {required}")

# Provider-media journeys share one test-only pre-navigation DOM harness. It waits
# for the application's visible ready state and then exercises the real consent and Connect controls.
# Production exposes no bootstrap event or alternate transport path for the harness.
for provider_e2e, label in (
    (voice_e2e, "Owner Lab voice"),
    (expressive_e2e, "Owner Lab Expressive"),
):
    for required in (
        'import { installProviderAutoConnect } from "./provider-bootstrap.js";',
        "await installProviderAutoConnect(page);",
    ):
        if required not in provider_e2e:
            raise SystemExit(f"{label} provider E2E pre-navigation connect harness missing: {required}")
if 'toHaveAttribute("data-vpr-provider-auto-connect", "clicked")' not in expressive_e2e:
    raise SystemExit("Owner Lab Expressive E2E must expose the provider autoconnect checkpoint")

for required in (
    'document.getElementById("connect")',
    'document.getElementById("consent")',
    'document.getElementById("status")',
    '"Готов к подключению"',
    "consent.checked = true",
    "connect.click()",
    "MutationObserver",
):
    if required not in provider_bootstrap:
        raise SystemExit(f"Owner Lab provider bootstrap harness missing canonical DOM path: {required}")
for forbidden in ("vpr:bootstrap-ready", "__vprBootstrap", "afterApiResponse"):
    if forbidden in provider_bootstrap:
        raise SystemExit(f"Owner Lab provider bootstrap harness depends on forbidden production orchestration: {forbidden}")

for provider_e2e, label in (
    (voice_e2e, "Owner Lab voice"),
    (expressive_e2e, "Owner Lab Expressive"),
):
    for forbidden in ("vpr:bootstrap-ready", "__vprBootstrap", 'page.locator("#consent").check()'):
        if forbidden in provider_e2e:
            raise SystemExit(f"{label} provider E2E contains forbidden post-navigation/bootstrap orchestration: {forbidden}")

for required in (
    "export const publishBootstrap",
    "export const whenBootstrap",
    "BOOTSTRAP_CSRF_MISSING",
):
    if required not in bootstrap_context:
        raise SystemExit(f"Owner Lab bootstrap context contract missing: {required}")
if 'from "./bootstrap-context.js"' not in app:
    raise SystemExit("Owner Lab application must publish bootstrap through the shared context")
if 'from "/bootstrap-context.js"' not in (UI / "reference-capture.js").read_text(encoding="utf-8"):
    raise SystemExit("Owner Lab reference capture must consume the shared bootstrap context")
for forbidden in ("__vprBootstrap", "vpr:bootstrap-ready", "vprProviderAutoConnect"):
    if forbidden in app:
        raise SystemExit(f"Owner Lab production bootstrap contains forbidden test shortcut: {forbidden}")


for required in (
    "run_owner_lab_voice_backend.py",
    "voice-provider-fixture.mjs",
):
    if required not in voice_config:
        raise SystemExit(f"Owner Lab voice browser config missing: {required}")

for required in (
    "VPR_OWNER_LAB_STT_PROVIDER",
    "VPR_OWNER_LAB_LLM_PROVIDER",
    "VPR_DID_ENDPOINT",
    "cargo",
):
    if required not in voice_launcher:
        raise SystemExit(f"Owner Lab voice launcher missing: {required}")

for required in (
    "/v1/audio/transcriptions",
    "/v1/listen",
    "alternatives",
    "/v1/chat/completions",
    "text/event-stream",
    "/agents/",
    "fluent: true",
    "interrupt_enabled: true",
):
    if required not in voice_provider:
        raise SystemExit(f"Owner Lab voice provider fixture missing: {required}")

for required in (
    'error?.code === "EPIPE"',
    'error?.code === "ECONNRESET"',
    'socket.on("error"',
    'socket.on("close"',
    "socket.destroyed",
    "socket.writableEnded",
):
    if required not in voice_provider:
        raise SystemExit(f"Owner Lab STT fixture missing cancellation-safe websocket lifecycle: {required}")

voice_test_sources = (voice_e2e, voice_media_fixture, voice_journey_driver, voice_journey_contract)
for forbidden in (
    "afterApiResponse",
    "prepareVoiceCaptureFakes",
    "__vprStartVoicePlayback",
):
    if any(forbidden in source for source in voice_test_sources):
        raise SystemExit(f"Owner Lab Voice test stack contains forbidden response-driven orchestration: {forbidden}")
for required in (
    "__vprSetPeerConnectionState",
    "vprFixtureAction",
    '["connected", "disconnected", "failed"]',
):
    if required not in voice_media_fixture:
        raise SystemExit(f"Owner Lab voice reconnect fixture contract missing: {required}")

for required in (
    "stream/started",
    "async setLocalDescription()",
    'this.connectionState = "connected"',
    "publishRemoteAudioTrack?.()",
    'payload.includes("stream/interrupt")',
    "async getUserMedia(constraints)",
    "AudioWorkletNode: FakeAudioWorkletNode",
    "beginSyntheticPlayback",
    "UNEXPECTED_WEBRTC_CLIENT_COMMAND",
):
    if required not in voice_media_fixture:
        raise SystemExit(f"Owner Lab voice fake transport lifecycle missing: {required}")
if "navigator.mediaDevices" in voice_media_fixture:
    raise SystemExit("Owner Lab WebRTC fixture must never fall through to native media devices")


voice_goto = voice_e2e.find('await page.goto("/");')
if voice_goto < 0:
    raise SystemExit("Owner Lab Voice E2E must navigate the real browser page")
voice_after_goto = voice_e2e[voice_goto + len('await page.goto("/");'):]
if "page." in voice_after_goto:
    raise SystemExit("Owner Lab Voice E2E controller must use no Playwright/CDP page RPC after navigation")
expressive_goto = expressive_e2e.find('await page.goto("/");')
if expressive_goto < 0:
    raise SystemExit("Owner Lab Expressive E2E must navigate the real browser page")
expressive_after_goto = expressive_e2e[expressive_goto + len('await page.goto("/");'):]
for forbidden in (
    "page.locator(",
    "page.getByRole(",
    "page.getByLabel(",
    "page.evaluate(",
    "page.waitForTimeout(",
    "expect(page.",
):
    if forbidden in expressive_after_goto:
        raise SystemExit(
            f"Owner Lab Expressive E2E must not issue post-navigation renderer RPC: {forbidden}"
        )

for required in ("/agents/", "authorization", "session_id", "ice_servers"):
    if required not in backend_provider:
        raise SystemExit(f"Owner Lab backend provider fixture missing: {required}")
for required in (
    "python3 tests/architecture/check_browser_contract.py",
    "npx playwright install --with-deps chromium",
    "npm run test:e2e",
    "npm run test:e2e:backend",
    "npm run test:e2e:voice",
    "npm run test:e2e:expressive",
    "dist/owner-capture.js",
    "dist/bootstrap-context.js",
    "dist/media-runtime.js",
    "dist/session-runtime-state.js",
):
    if required not in ci:
        raise SystemExit(f"Owner Lab CI missing browser-contract enforcement: {required}")

print("owner-lab-browser-contract: PASS")
