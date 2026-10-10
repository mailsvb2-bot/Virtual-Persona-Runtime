# Optional MetroTrance local TTS for RT0 (experimental)

This **does not** modify `mailsvb2-bot/metrotrance`. It reuses the installed
MetroTrance Python Qwen3-TTS or Chatterbox provider, voice profile and speech
quality selection directly from a separate local checkout. No model weights
or sample voices are copied into VPR.

## Scope / security

The adapter is an opt-in, loopback-only, bearer-token-protected local speech
service implementing the OpenAI speech wire shape: `POST /v1/audio/speech`
with `{"input":"...","response_format":"wav"}` -> `audio/wav`.

It uses `takes_per_chunk=1` to avoid MetroTrance's multi-take studio latency.
Inference is still synchronous and may be too slow on CPU. Measure it on real
hardware before claiming conversational readiness.

The bearer-protected bridge is a **synthesis provider only**, not a policy
authority. In RT0, Owner Lab selects a `TtsPort`, the canonical
`ActiveTurn.execute_tts` creates bounded PCM, and a *fresh* avatar permit
authorizes publication through a private Echo-only LiveKit sender. Echo's
PCM-only mode refuses its former direct HTTP TTS path. A revocation during
synthesis prevents publication; a failed/uncertain private send fences the
publisher and attempts STOP.

This is still **not** a guarantee of real audible playback or low latency:
the D-ID vendor sends media asynchronously, and local model speed must be
measured on the intended Windows hardware.

## Requirements

- Separate MetroTrance source checkout and its installed Python dependencies
  (`qwen-tts` or `chatterbox-tts`, plus their optional model dependencies)
- Prepared private MetroTrance reference voice at
  `<MetroTrance data>/voice/reference.wav`; Qwen also needs
  `voice/transcript.txt`
- Local model available; no implicit installation or download in VPR
- An explicit random token of at least 24 characters, kept server-private

In a dedicated terminal with MetroTrance's Python environment active:

```powershell
$env:METROTRANCE_ROOT = 'C:\path\to\metrotrance'
$env:VPR_METROTRANCE_ENGINE = 'qwen' # or chatterbox
$env:VPR_METROTRANCE_TTS_PORT = '8897'
$env:VPR_METROTRANCE_TTS_TOKEN = '<random-private-secret-at-least-24-characters>'
python scripts/metrotrance_tts_bridge.py
```

The listener is hard-bound to `127.0.0.1`. It rejects unknown paths,
unauthenticated calls, malformed payloads, non-WAV requests, text beyond
4096 UTF-8 bytes, and oversized audio. Request bodies and voices are not
logged. Temp WAVs are removed after each request; one inference at a time.

For an **isolated** local adapter test (no MetroTrance install needed):

```powershell
python -m unittest discover -s tests/e2e -p test_metrotrance_tts_bridge.py
```

## Select and switch providers

The VPR Owner Lab process accepts one mandatory primary and one optional
fallback TTS provider. Both endpoints/keys remain in the backend environment.
Provider identifiers are `metrotrance-qwen`,
`metrotrance-chatterbox`, `openai-speech`, or `elevenlabs`.

For the Qwen bridge above, launch Owner Lab from an environment containing:

```powershell
$env:VPR_OWNER_LAB_TTS_PROVIDER = 'metrotrance-qwen'
$env:VPR_OWNER_LAB_TTS_ENDPOINT = 'http://127.0.0.1:8897/v1/audio/speech'
$env:VPR_OWNER_LAB_TTS_API_KEY = '<same-private-secret-as-bridge>'
$env:VPR_OWNER_LAB_TTS_MODEL = 'metrotrance-qwen'
$env:VPR_OWNER_LAB_TTS_VOICE = 'local'
```

For a second Chatterbox backend, launch a separate bridge with
`VPR_METROTRANCE_ENGINE=chatterbox` and port `8898`, then set:

```powershell
$env:VPR_OWNER_LAB_TTS_ALT_PROVIDER = 'metrotrance-chatterbox'
$env:VPR_OWNER_LAB_TTS_ALT_ENDPOINT = 'http://127.0.0.1:8898/v1/audio/speech'
$env:VPR_OWNER_LAB_TTS_ALT_API_KEY = '<secret-for-second-bridge>'
$env:VPR_OWNER_LAB_TTS_ALT_MODEL = 'metrotrance-chatterbox'
$env:VPR_OWNER_LAB_TTS_ALT_VOICE = 'local'
```

With a selected canonical TTS provider, Owner Lab automatically requests
D-ID PCM-only Echo (unless `VPR_DID_ECHO_ENABLED=false` explicitly forbids
Echo, in which case startup fails closed). A separate provider Python
interpreter can be supplied by `VPR_DID_ECHO_PYTHON`; otherwise `python`
is used. The stored D-ID, STT and LLM credentials remain separate.

When two providers are configured, the Owner Lab UI exposes a selector.
`GET /api/tts/providers` returns only the identifiers. The browser submits
`POST /api/tts/select` with the existing CSRF header. The backend rejects
unknown models, visitor changes and changes while a voice turn is busy.
Switching preserves the current avatar session and takes effect on the next
utterance; it does not replay or migrate already submitted audio. For
security the browser never sends endpoint URLs or provider credentials.

The former full D-ID browser-mediated text-speaking mode is not unlocked by
this selector. Re-enabling it requires independent vendor-side revocation
proof, not just an alternative TTS configuration.

## Release/acceptance conditions not yet established

- A full `cargo check --workspace --all-targets --locked`, format, clippy,
  Rust tests, real Echo worker tests and browser E2E must be green on this PR.
- Verify real D-ID Expressive audible speech, first audio latency, A/V drift,
  interruption during TTS, STOP receipt and provider switching in both
  directions on the user's Windows system.
- Vendor stream-close acknowledgement is **not** proof that audio finished
  playing; do not publish fake completion evidence.
- TTS usage/cost outcome must be included in end-to-end evidence before
  billing-sensitive production use.
