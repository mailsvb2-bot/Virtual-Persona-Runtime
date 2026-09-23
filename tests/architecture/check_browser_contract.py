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
EXPRESSIVE_E2E = UI / "e2e" / "backend-expressive-journey.spec.ts"
EXPRESSIVE_CONFIG = UI / "playwright.expressive.config.ts"
EXPRESSIVE_LAUNCHER = ROOT / "tests" / "e2e" / "run_owner_lab_expressive_backend.py"
APP = UI / "src" / "app.ts"
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
expressive_e2e = EXPRESSIVE_E2E.read_text(encoding="utf-8")
expressive_config = EXPRESSIVE_CONFIG.read_text(encoding="utf-8")
expressive_launcher = EXPRESSIVE_LAUNCHER.read_text(encoding="utf-8")
app = APP.read_text(encoding="utf-8")
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


for required in (
    "voice_attempts",
    "Привет из браузера",
    "Что думает владелец?",
    "Visitor permissions do not expose owner-reviewed personal context",
    "Bearer voice-stt-e2e-secret",
    "Bearer voice-llm-e2e-secret",
    "Basic voice-avatar-e2e-secret",
    "av_sync_proven",
    "web_rtc_estimated_playout_timestamp",
    "absolute_offset_millis",
    "stream/started",
    "stream/interrupt",
    "videoId",
    "interruption_stopped",
    "__vprSetPeerConnectionState",
    "reconnect_restored",
    '"disconnected"',
    '"connected"',
    "PROVIDER_UNAVAILABLE",
    "Спровоцируй отказ провайдера",
    "Восстановление после отказа",
    'failure_code: "PROVIDER_UNAVAILABLE"',
    'getByRole("button", { name: "Прервать", exact: true })',
):
    if required not in voice_e2e:
        raise SystemExit(f"Owner Lab voice browser proof missing: {required}")

for required in (
    "LiveKit согласован",
    "canonical_playback_proven",
    "av_sync_proven",
    "did.speak",
    "did.interrupt",
    "__vprExpressiveDisconnect",
    "/v2/agents/voice-e2e-expressive-agent/sessions",
    "#metric-stt",
    "#metric-llm-first",
    "#metric-av-sync",
    "#metric-playback",
    "#metric-cost",
    '"/v1/listen"',
    '"model=nova-3"',
    '"language=ru"',
    '"reasoning_effort":"none"',
    '"max_tokens":96',
    '"model":"deepseek-flash"',
):
    if required not in expressive_e2e:
        raise SystemExit(f"Owner Lab Expressive browser proof missing: {required}")

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

for required in (
    "realtimeReadiness = { control: false, audio: false, video: false }",
    "realtimeReadiness.control",
    "realtimeReadiness.audio",
    "realtimeReadiness.video",
):
    if required not in app:
        raise SystemExit(f"Owner Lab modality readiness split missing: {required}")
if "realtimeTransportReady" in app:
    raise SystemExit("Owner Lab must not collapse control/audio/video readiness into one flag")
if '&& realtimeReadiness.audio' not in app:
    raise SystemExit("Owner Lab voice readiness must require the realtime audio modality")
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
    if required not in backend_e2e or required not in voice_e2e:
        raise SystemExit(f"Owner Lab owner/visitor evidence export proof missing: {required}")

for source, required in (
    (app, "const AV_SYNC_SAMPLE_COUNT = 3;"),
    (session_evidence, "pub const RT0_AV_SYNC_SAMPLES_PER_REQUEST: u32 = 3;"),
    (rt0_release_spec, "Exactly three samples, sequence-numbered `1..=3`, are required"),
):
    if required not in source:
        raise SystemExit(f"Owner Lab A/V sync sample-count contract missing: {required}")

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
):
    if required not in ci:
        raise SystemExit(f"Owner Lab CI missing browser-contract enforcement: {required}")

print("owner-lab-browser-contract: PASS")
