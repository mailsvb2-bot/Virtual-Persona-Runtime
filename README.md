# Virtual Persona Runtime

Portable identity and presence runtime for digital persons.

> One identity. Any supported intelligence. Any supported voice. Any supported embodiment. Anywhere the published compatibility contract allows.

This repository is governed by `docs/CANON.md` and develops through bounded Release Trains beginning with RT0 — Feasibility & Wow Proof.

## Status

Bootstrap / RT0 foundation. No capability is `PRODUCTION_READY` until exact-candidate evidence satisfies the Canon.

## RT0 Owner Lab

`vpr-owner-lab` is the experimental loopback-only browser path for proving realtime avatar presence through the canonical runtime. The runtime contract is provider-neutral: an avatar adapter negotiates a supported realtime transport, while Persona identity, authority, output evidence, and conversation state remain independent of that provider. It is not a production-ready avatar claim and remains fail-closed unless both backend egress and explicit in-browser consent are enabled.

The current D-ID adapter automatically selects the transport exposed by the configured presenter: legacy presenters use the Agents Streams WebRTC control plane, while `expressive` presenters use D-ID V2 sessions and a session-scoped LiveKit connection. D-ID-specific endpoints, topics, session identifiers, and credentials remain adapter details rather than canonical Persona state.

Build the browser bundle:

```bash
cd crates/vpr-owner-lab/ui
npm ci --ignore-scripts
npm run build
cd ../../..
```

Run the local lab with one explicitly selected supported realtime-avatar provider. For the self-hosted provider:

```bash
VPR_OWNER_LAB_AVATAR_PROVIDER=local-open-source \
VPR_LOCAL_AVATAR_ENDPOINT='https://avatar.example.test' \
VPR_LOCAL_AVATAR_API_TOKEN='<avatar-token>' \
VPR_OWNER_LAB_ALLOW_EGRESS=true \
cargo run -p vpr-owner-lab
```

Or select D-ID explicitly:

```bash
VPR_OWNER_LAB_AVATAR_PROVIDER=did \
VPR_DID_API_KEY='<key>' \
VPR_DID_AGENT_ID='<agent-id>' \
VPR_OWNER_LAB_ALLOW_EGRESS=true \
cargo run -p vpr-owner-lab
```

Then open `http://127.0.0.1:8787`. `VPR_OWNER_LAB_PORT` is an optional override. `VPR_DID_ENDPOINT` and `VPR_DID_FLUENT=true` apply only when D-ID is selected. Provider API credentials remain in the Rust process. The browser receives only the session-scoped signaling or transport material required by the negotiated realtime transport.

D-ID and `local-open-source` are peer adapters behind the same `RealtimeAvatarPort`; neither is canonical Persona or Appearance identity, and no adapter may silently take over when another provider is selected.


### Shared voice engine configuration (RT0 integration)

The runtime already defines the provider-neutral `TtsPort` and has OpenAI Speech
and ElevenLabs adapters. D-ID Echo's private sender can now use the **shared**
OpenAI-compatible WAV speech endpoint configuration. Avatar viewers do not
configure a voice, download Python or receive a TTS API key; the operator
configures the voice service once on the backend:

```text
VPR_VOICE_ENGINE_ENDPOINT=https://speech.example.com/v1/audio/speech
VPR_VOICE_ENGINE_API_KEY=<server-only credential>
VPR_VOICE_ENGINE_MODEL=<speech model>
VPR_VOICE_ENGINE_VOICE=<reviewed voice>
VPR_DID_ECHO_ENABLED=true
VPR_DID_ECHO_PYTHON=<server Python with LiveKit RTC>
```

The selected service must implement the OpenAI-style `POST` body with
`input`, `model`, `voice`, `response_format: "wav"`, returning bounded
PCM WAV. A locally hosted compatible service can be addressed via loopback
HTTP; a remote service requires HTTPS. The older `VPR_DID_ECHO_TTS_*` variables
remain supported, but if both old and new values exist and disagree, startup
fails closed rather than picking a different voice silently.

This is **one backend voice configuration**, not a claim that the existing
D-ID integration has already switched to the general `TtsPort` scheduler.
The provider-neutral TTS adapters are available for future avatar transports.
Before promoting RT0, measure naturalness, time to first audible word, A/V
alignment, interruption and continuation on real D-ID. The shared engine
does not itself prove audible playback completion or remove provider charges.
Do not ask end users to configure the voice service.

### Opt-in D-ID Echo: private LiveKit audio sender (experimental)

The default Expressive/browser `did.speak` path is unchanged. To test the server-owned Echo transport instead, use a D-ID **v4 expressive** agent and explicitly enable all of these process-scoped settings in addition to the usual D-ID, STT/LLM, consent and egress configuration:

```text
VPR_DID_ECHO_ENABLED=true
VPR_DID_ECHO_PYTHON=<path to a Python 3 interpreter with the livekit package installed>
VPR_DID_ECHO_TTS_ENDPOINT=https://api.openai.com/v1/audio/speech
VPR_DID_ECHO_TTS_API_KEY=<private TTS key>
VPR_DID_ECHO_TTS_MODEL=<speech model>
VPR_DID_ECHO_TTS_VOICE=<speech voice>
```

Install the SDK **into that specific interpreter** using `python -m pip install livekit` before launching Owner Lab. Python/LiveKit is a runtime prerequisite for this opt-in experimental sender; it is **not** bundled into the Windows RT0 launcher. For the new code, launch Owner Lab manually on the selected clean PR candidate, rather than using the Windows launcher that intentionally updates to `main` and clears transient `VPR_*` overrides.

The Rust backend launches a private local worker; it sends TTS-generated bounded WAV utterances to `did.audio-stream` and uses `did.interrupt` for STOP. Only the viewer-scoped LiveKit token reaches the browser; `echo_token`, TTS key and audio remain server-private. STOP is generation-scoped and can run without waiting for an STT/LLM worker's engine mutex. Invalid configuration fails closed; this transport is never chosen as an implicit fallback. The Python worker receives credentials over a private subprocess boundary and never writes them to stdout.

**Not RT0 exit evidence:** neither `did.audio-stream` success nor video-generation completion proves the entire reply was audibly played. Real D-ID owner/visitor sessions, authoritative playback confirmation, RTP A/V sync, human acceptance and cost still require exact-candidate proof. For security, do not mark RT0 ready based only on mocked CI.


Optional push-to-talk voice conversation can be enabled without changing the avatar-only path. Configure one STT provider and one LLM provider together:

```bash
VPR_OWNER_LAB_STT_PROVIDER=openai-transcription \
VPR_OWNER_LAB_STT_ENDPOINT='https://api.openai.com/v1/audio/transcriptions' \
VPR_OWNER_LAB_STT_API_KEY='<stt-key>' \
VPR_OWNER_LAB_STT_MODEL='<stt-model>' \
VPR_OWNER_LAB_LLM_PROVIDER=openai-compatible \
VPR_OWNER_LAB_LLM_ENDPOINT='<chat-completions-endpoint>' \
VPR_OWNER_LAB_LLM_API_KEY='<llm-key>' \
VPR_OWNER_LAB_LLM_MODEL='<llm-model>' \
VPR_OWNER_LAB_AVATAR_PROVIDER=local-open-source \
VPR_LOCAL_AVATAR_ENDPOINT='https://avatar.example.test' \
VPR_LOCAL_AVATAR_API_TOKEN='<avatar-token>' \
VPR_OWNER_LAB_ALLOW_EGRESS=true cargo run -p vpr-owner-lab
```

STT provider values are `openai-transcription` (alias `openai`) and `deepgram`. LLM provider values are `openai-compatible` (alias `openai`), `deepseek`, `anthropic`, and `gemini`. The browser captures push-to-talk audio with `AudioWorklet`, converts it locally to mono PCM S16LE at 16 kHz, and sends the binary utterance to the loopback backend. The backend runs one canonical authorized turn through STT -> LLM -> realtime avatar and returns transcript/reply plus latency/usage evidence. Raw microphone PCM is not returned as evidence, and provider credentials are never sent to the browser. Voice mode is still experimental and does not prove credentialed RT0 exit criteria.

### Windows provider credentials

On Windows, configure the RT0 provider stack once and store it in **Windows Credential Manager** for the current Windows user. This replaces the old CMD-only `set` workflow, whose values disappeared when that shell closed.

If the CMD window that already contains the old `set VPR_...` values is still open, migrate those values without re-entering the keys:

```powershell
cargo run -p vpr-owner-lab --bin vpr-provider-credentials -- import-env
```

Otherwise run the secure setup once:

```powershell
cargo run -p vpr-owner-lab --bin vpr-provider-credentials -- set
```

The setup first asks which avatar provider to use. A D-ID profile asks for the D-ID API key and agent ID; a `local-open-source` profile asks only for the local worker endpoint and service token and does not require D-ID credentials. Both profiles then collect the RT0 Deepgram and DeepSeek credentials. Secret values are entered without terminal echo, and the selected avatar provider is probed before the profile is saved. For D-ID, the raw key format is `API_USERNAME:API_PASSWORD`; an accidental leading `Basic ` prefix is stripped before storage.

To replace only D-ID credentials while preserving the stored Deepgram and DeepSeek keys:

```powershell
cargo run -p vpr-owner-lab --bin vpr-provider-credentials -- set-did
```

The replacement is probed before it overwrites the existing D-ID credentials. The probe first validates the API key against D-ID's read-only account-level `GET /credits` endpoint, then validates access to the configured Agent/runtime path. A failed probe leaves the previous secure profile unchanged.

Check configuration without revealing keys:

```powershell
cargo run -p vpr-owner-lab --bin vpr-provider-credentials -- status
```

Remove the stored profile:

```powershell
cargo run -p vpr-owner-lab --bin vpr-provider-credentials -- clear
```

Owner Lab and `vpr-live-proof` use this Windows credential profile when the corresponding `VPR_*` environment configuration is absent. Explicit environment avatar selection is authoritative: a complete selected/inferred avatar environment overrides the stored avatar binding, while partial or ambiguous avatar environment fails closed instead of silently reverting to the stored provider. For hermetic CI or a deliberate environment-only run, set `VPR_PROVIDER_CREDENTIAL_SOURCE=environment`; that disables Windows Credential Manager lookup for the process and fails closed if the environment is incomplete. API-key values are never printed or included in provider descriptors or evidence.

For the Windows RT0 operator path, use the safe launcher:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\windows-owner-lab-restart.ps1
```

For **exact-candidate RT0 browser evidence**, use the stricter evidence mode:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\windows-owner-lab-restart.ps1 -Rt0Evidence
```

The evidence mode fails before provider launch unless Firefox 142+ is installed, then opens Owner Lab in that Firefox instance. This is required because the RT0 A/V-sync contract accepts only WebRTC `RTCInboundRtpStreamStats.estimatedPlayoutTimestamp`; Chromium/Edge do not currently expose that measurement reliably enough for this gate. Normal development launches keep using the default browser.

The launcher preserves the canonical reviewed Persona when it can do so losslessly, stops the stale listener on the selected port, fast-forwards `main`, rebuilds Owner Lab, clears inherited provider `VPR_*` overrides so the secure Windows Credential Manager profile is authoritative, runs `probe-avatar` against the avatar provider selected by that profile, pins `VPR_OWNER_LAB_PORT`, enables egress both through the process environment and `--allow-egress`, verifies that the expected `vpr-owner-lab.exe` owns that port, and checks both bootstrap and runtime status before opening the browser. D-ID-specific diagnostics run only when D-ID is the selected adapter; `local-open-source` launches do not require or probe D-ID. This prevents stale environment credentials, an old process, a hidden provider switch, or a wrong-port Owner Lab instance from masquerading as the canonical launch path.

For deliberate manual runs, the lower-level process-scoped opt-in is still available:

```powershell
cargo run -p vpr-owner-lab -- --allow-egress
```

Closing the process returns to the fail-closed default; egress permission is not stored with provider credentials. The existing `VPR_OWNER_LAB_ALLOW_EGRESS=true` environment variable remains supported for automation.