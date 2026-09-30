# Local open-source realtime avatar worker

This document freezes the RT0-facing contract for a self-hosted GPU avatar worker. The worker is
not a second runtime or Persona brain. VPR remains authoritative for Persona, policy, STT, LLM,
turn lifecycle, interruption authority, and evidence. The GPU worker only renders authorized text
into realtime audio/video.

## Target stack

Initial candidate:

- NVIDIA L4 24 GB
- Ubuntu 24.04
- CUDA-compatible container runtime
- MuseTalk for realtime lip-sync
- LivePortrait only if additional head/face motion is needed after baseline latency is proven
- local TTS inside the worker (Piper or another replaceable local engine)
- WebRTC egress from worker to browser
- HTTPS control plane from Owner Lab to worker

The rendering implementation is replaceable. Owner Lab depends only on the HTTP/WebRTC contract
below.

## VPR configuration

```text
VPR_OWNER_LAB_AVATAR_PROVIDER=local-open-source
VPR_LOCAL_AVATAR_ENDPOINT=https://avatar.example.ru
VPR_LOCAL_AVATAR_API_TOKEN=<secret>
```

STT and LLM configuration remains unchanged. D-ID credentials may remain stored as fallback but are
not read by the local adapter when this provider is selected.

Remote plaintext HTTP is rejected. HTTP is accepted only for loopback development.

## Worker API v1

All requests require:

```text
Authorization: Bearer <VPR_LOCAL_AVATAR_API_TOKEN>
Content-Type: application/json
```

### Create session

`POST /v1/avatar/sessions`

Request:

```json
{}
```

Response:

```json
{
  "id": "session-id",
  "offer": {
    "type": "offer",
    "sdp": "..."
  },
  "ice_servers": [
    {
      "urls": ["stun:...", "turn:..."],
      "username": "...",
      "credential": "..."
    }
  ]
}
```

### Browser answer

`POST /v1/avatar/sessions/{id}/answer`

```json
{"type":"answer","sdp":"..."}
```

### ICE

`POST /v1/avatar/sessions/{id}/ice`

```json
{
  "candidate": "candidate:...",
  "sdpMid": "0",
  "sdpMLineIndex": 0
}
```

A null candidate is the end-of-candidates marker.

### Speak

`POST /v1/avatar/sessions/{id}/speak`

```json
{"text":"authorized VPR reply"}
```

The worker must synthesize speech locally and stream synchronized audio/video over the already
negotiated WebRTC session. It must not call an LLM or mutate Persona content.

### Interrupt

`POST /v1/avatar/sessions/{id}/interrupt`

```json
{}
```

The worker must stop queued/current TTS and rendering quickly enough for VPR barge-in tests.

### Close

`DELETE /v1/avatar/sessions/{id}`

Must release GPU/session resources idempotently.

## Security boundary

The worker must never receive provider API keys for STT/LLM, the full reviewed Persona profile, or
historical conversation storage. It receives only the authorized reply text needed for current
playback.

The bearer token is a service credential and must be stored outside the repository. TLS is required
for any non-loopback deployment.

## RT0 acceptance measurements

Before replacing D-ID for RT0 evidence, measure at least:

- session creation latency
- first video frame latency
- text accepted -> first audible sample
- sustained video FPS
- A/V sync offset
- interruption latency
- GPU memory usage
- one-hour stability
- behavior after worker restart/disconnect
- exact cost per active hour on the selected Timeweb GPU

Do not promote maturity merely because the adapter compiles. A real owner session over the exact
GPU candidate is required.
