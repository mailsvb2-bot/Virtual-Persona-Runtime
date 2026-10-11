# RT0: lightweight offline Piper speech (experimental)

This is an **opt-in** local speech source, not a new policy owner or independent
avatar publisher. Piper -> bearer-protected loopback speech endpoint -> canonical
`TtsPort` / `ActiveTurn.execute_tts` -> fresh avatar permit -> private
D-ID Echo PCM sender. The browser cannot choose a model or endpoint.

The initial voice is deliberately standard; **no voice cloning**, MetroTrance
checkout, reference.wav, transcript.txt, Qwen model, or GPU is required.

## Windows prerequisites and voice

Use the Python interpreter that will **run the Piper bridge**. Keep it distinct
from the existing D-ID Echo worker's Python environment if necessary.

```powershell
py -3.11 -m venv .vpr-piper
.\.vpr-piper\Scripts\python.exe -m pip install piper-tts
New-Item -ItemType Directory -Force -Path .local-piper | Out-Null
Push-Location .local-piper
..\.vpr-piper\Scripts\python.exe -m piper.download_voices ru_RU-irina-medium
Pop-Location
```

This downloads a small Russian voice and its configuration, once. Review the
selected voice's MODEL_CARD/license before public or commercial deployment.
Both `ru_RU-irina-medium.onnx` and `ru_RU-irina-medium.onnx.json` must
exist. The bridge loads local files only and cannot download weights during a
request. Keep the model and virtualenv **out of Git**; do not record client
text or secrets in diagnostics.

## Bridge terminal

Generate a fresh private token of at least 24 characters. Do not copy a demo
token into production. In PowerShell at the VPR repository root:

```powershell
$env:VPR_PIPER_MODEL = (Resolve-Path '.local-piper\ru_RU-irina-medium.onnx').Path
$env:VPR_METROTRANCE_TTS_PORT = '8897'
$env:VPR_METROTRANCE_TTS_TOKEN = '<private-random-token-minimum-24-chars>'
.\.vpr-piper\Scripts\python.exe scripts\piper_tts_bridge.py
```

The inherited token variable name is retained for compatibility with the
existing secure loopback HTTP handler, **not** because Piper runs MetroTrance.
The listener binds only `127.0.0.1`, requires the bearer token, refuses
mismatched `model`, enforces bounded requests and rejects non-PCM WAV.

## Owner Lab terminal

Set exactly the **same** private bearer token; it is never browser-visible.
The existing Windows credential profile supplies D-ID, Deepgram, and DeepSeek.

```powershell
$env:VPR_OWNER_LAB_TTS_PROVIDER = 'piper'
$env:VPR_OWNER_LAB_TTS_ENDPOINT = 'http://127.0.0.1:8897/v1/audio/speech'
$env:VPR_OWNER_LAB_TTS_API_KEY = '<same-token-as-bridge>'
$env:VPR_OWNER_LAB_TTS_MODEL = 'piper'
$env:VPR_OWNER_LAB_TTS_VOICE = 'local'
```

For **test branch evidence** use the existing strict Windows launcher with a
clean Git clone and Firefox, because the ordinary launcher deliberately
switches to `main` and would not test this unmerged branch:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\windows-owner-lab-restart.ps1 -Rt0Evidence
```

Do not disable Echo, STOP/revocation, or browser evidence requirements to
make a voice sample play. The selected D-ID avatar needs real PCM-only Echo.
The owner must verify D-ID Expressive playback, interrupt/STOP, visitor access,
measured latency/AV sync, cost, and human review before RT0 exit.

## CI and limitations

```powershell
python -m unittest discover -s tests/e2e -p test_piper_tts_bridge.py
```

The required Linux CI runs Piper's hermetic model/PCM/HTTP authentication
regressions without downloading voices or importing Piper as an installed
package. CI success proves the contract, **not** real synthesis speed or
audible video playback. Do not merge solely because checks are green.

Official engine: https://github.com/OHF-Voice/piper1-gpl
Official voice models: https://huggingface.co/rhasspy/piper-voices
