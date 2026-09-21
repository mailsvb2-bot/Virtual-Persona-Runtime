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

STT provider values are `openai-transcription` (alias `openai`) and `deepgram`. LLM provider values are `openai-compatible` (alias `openai`), `anthropic`, and `gemini`. The browser captures push-to-talk audio with `AudioWorklet`, converts it locally to mono PCM S16LE at 16 kHz, and sends the binary utterance to the loopback backend. The backend runs one canonical authorized turn through STT -> LLM -> realtime avatar and returns transcript/reply plus latency/usage evidence. Raw microphone PCM is not returned as evidence, and provider credentials are never sent to the browser. Voice mode is still experimental and does not prove credentialed RT0 exit criteria.
