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

**This adapter is NOT an authority boundary.** It must be called only through
VPR's existing authorized TTS runtime (`ActiveTurn.execute_tts`) as the final
design. The present D-ID Echo Python worker can already call this HTTP shape,
but merely pointing Echo at this service **does not establish** the canonical
TTS-permit, cancellation/STOP, outcome accounting or real playback evidence.
Do not merge into an accepted R0 configuration until those connections and
tests are complete. A bearer token is not a substitute for runtime policy.

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

## Remaining RT0 integration work

1. Route speech through `ActiveTurn.execute_tts` and its policy/cancellation
   permit, rather than invoking the HTTP service directly from the Echo worker.
2. Convey bounded PCM/WAV to the existing Echo publisher without disclosing
   publisher tokens or private credentials to browser JavaScript.
3. Fence the sender on revoke, interrupt and session close, including synthesis
   in progress; require vendor STOP evidence, not only HTTP completion.
4. Add live measurements for first audible audio, queueing and A/V synchronization
   with the exact Windows RT0 candidate.
