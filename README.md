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

Run the local lab with a D-ID Agents Streams credential:

```bash
VPR_DID_API_KEY='<key>' \
VPR_DID_AGENT_ID='<agent-id>' \
VPR_OWNER_LAB_ALLOW_EGRESS=true \
cargo run -p vpr-owner-lab
```

Then open `http://127.0.0.1:8787`. `VPR_DID_ENDPOINT` and `VPR_OWNER_LAB_PORT` are optional overrides. `VPR_DID_FLUENT=true` applies only to the legacy D-ID Streams path and may be enabled for compatible presenters. Provider API credentials remain in the Rust process. The browser receives only the session-scoped signaling or transport material required by the negotiated realtime transport.

D-ID is one adapter, not the architecture center. Future avatar providers may implement the same `RealtimeAvatarPort` using WebRTC, LiveKit, or another explicitly modeled transport without changing canonical Persona identity or authority.

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
VPR_DID_API_KEY='<key>' VPR_DID_AGENT_ID='<agent-id>' \
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

The setup asks for the D-ID API key, D-ID agent ID, Deepgram API key, and DeepSeek API key. Secret values are entered without terminal echo. D-ID is validated before the profile is saved. The raw D-ID key format is `API_USERNAME:API_PASSWORD`; an accidental leading `Basic ` prefix is stripped before storage. The saved profile selects D-ID with the historical RT0 `VPR_DID_FLUENT` behavior (disabled/unset), Deepgram `nova-3`, and DeepSeek `deepseek-flash`.

To replace only D-ID credentials while preserving the stored Deepgram and DeepSeek keys:

```powershell
cargo run -p vpr-owner-lab --bin vpr-provider-credentials -- set-did
```

The replacement is probed before it overwrites the existing D-ID credentials. The probe first validates the API key against a read-only account-level D-ID endpoint, then validates access to the configured Agent/runtime path. A failed probe leaves the previous secure profile unchanged.

Check configuration without revealing keys:

```powershell
cargo run -p vpr-owner-lab --bin vpr-provider-credentials -- status
```

Remove the stored profile:

```powershell
cargo run -p vpr-owner-lab --bin vpr-provider-credentials -- clear
```

Owner Lab and `vpr-live-proof` automatically fall back to this Windows credential profile when matching `VPR_*` environment variables are absent. Explicit environment variables still take priority, so CI and deliberate per-process overrides keep their existing behavior. API-key values are never printed or included in provider descriptors or evidence.

For the Windows RT0 operator path, use the safe launcher:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\windows-owner-lab-restart.ps1
```

The launcher preserves the canonical reviewed Persona when it can do so losslessly, stops the stale listener on the selected port, fast-forwards `main`, rebuilds Owner Lab, clears inherited provider `VPR_*` overrides so the secure Windows Credential Manager profile is authoritative, runs a safe D-ID credential/agent preflight (metadata lookup first; if metadata access is forbidden, it may create and immediately close one legacy stream to verify the historical runtime path), pins `VPR_OWNER_LAB_PORT`, enables egress both through the process environment and `--allow-egress`, verifies that the expected `vpr-owner-lab.exe` owns that port, and checks both bootstrap and runtime status before opening the browser. This prevents stale environment credentials, an old process, or a wrong-port Owner Lab instance from masquerading as the canonical launch path.

For deliberate manual runs, the lower-level process-scoped opt-in is still available:

```powershell
cargo run -p vpr-owner-lab -- --allow-egress
```

Closing the process returns to the fail-closed default; egress permission is not stored with provider credentials. The existing `VPR_OWNER_LAB_ALLOW_EGRESS=true` environment variable remains supported for automation.
