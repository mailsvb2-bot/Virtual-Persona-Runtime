# Windows RT0: one-click Piper voice (development candidate)

**Goal:** a local Russian-speaking D-ID Echo avatar with no Qwen downloads,
voice cloning, separate MetroTrance installation or manually launched TTS server.

## One-click launch

From a **clean Git clone** of the selected RT0 candidate, double-click the **Piper-only fallback**:

`scripts\start-rt0-piper.cmd`

The Piper-only launcher delegates to the existing canonical Windows RT0 evidence launcher
with `-Piper -Rt0Evidence`. It does not switch branches, update the checkout
or weaken the Firefox strict-evidence requirement. Prerequisites still include
the existing Rust toolchain, Windows secure provider profile (D-ID, Deepgram,
DeepSeek), Python 3.11 registered as `py -3.11`, and an internet connection
for the *first* optional voice download. The root `START_VPR_WITH_VOICE.cmd` now selects the installed female
Microsoft Irina Desktop voice; see `docs/WINDOWS_NATIVE_VOICE_RT0.md`.

This is **not yet a packaged end-user
installer**; it makes the existing Windows developer RT0 lab one click.

On first use, it:

1. Creates a Piper-only virtual environment under
   `%LOCALAPPDATA%\Virtual-Persona-Runtime\Piper\venv`.
2. Installs the fixed engine version `piper-tts==1.3.0`, separate from the
   existing Echo Python interpreter.
3. Downloads the selected Russian voice (approximately 63 MB) from the pinned
   `rhasspy/piper-voices` v1.0.0 revision, verifying SHA-256 and Russian 22,050 Hz configuration:
   - Default female `ru_RU-irina-medium`: `8ff38212d23da300bbe3705c645e6e5b9475f0bfde01558eb17813e22acaaaaa`
   - Alternative male `ru_RU-denis-medium`: `15fab56e11a097858ee115545d0f697fc2a316c41a291a5362349fb870411b0a`
   Select via `-PiperVoice` or an explicit interim VoiceProfile.
   A failed or interrupted download cannot replace a validated voice.
4. Starts only the local `127.0.0.1:8897` speech bridge with a fresh
   cryptographically random, per-launch bearer token. It rejects port
   conflicts instead of using or killing unrecognized services.
5. Requests a small authenticated warmup WAV, then configures the canonical
   Owner Lab `TtsPort` for `piper` and launches the existing D-ID Echo PCM
   sender path.
6. Opens Firefox and supervises the Piper subprocess; on normal Owner Lab exit
   it stops the exact Piper child it launched, without a persistent token file.

The model and environment are cached outside Git. The second launch should
not download them again. The Windows PowerShell console remains open while
the lab is running because it supervises the local TTS worker.

## Startup latency and readiness (RT0)

The Piper bridge loads the installed model and synthesizes a fixed Russian warmup
sentence **before** binding its loopback listener. An invalid WAV or warmup
failure prevents the service from starting; readiness is never fabricated.

After successful warmup, `GET http://127.0.0.1:8897/health` returns only
`{"ready": true, "rtf": <measured value>}`. RTF is elapsed synthesis time
divided by generated audio duration, not an end-to-end latency estimate.
The endpoint contains no bearer token, input transcript, voice audio or
model path. It is loopback-only. The existing Windows launcher independently
verifies listener ownership and the authenticated speech endpoint.

This warmup reduces cold-start exposure but does **not** demonstrate the
QualityContract first-audio p95: collect real browser evidence before asserting
a pass. The original dev-only Denis download remains separate from any final
rights-cleared female voice choice. The requested Irina dataset's licensing is
unverified and it must **not** be promoted into a commercial/release preset
without an independent rights review.

## Boundaries and license

The bridge is **only a synthesis provider**. Browser clients cannot choose
endpoint, provider credentials or model. The server-owned canonical
`ActiveTurn.execute_tts()` creates bounded PCM; private Echo publication
still requires a fresh permit. Cancellation/revocation must stop publication.
No normal launch silently installs the voice: provisioning is activated only
by `-Piper` or the explicit one-click entrypoint.

The **Piper engine is GPLv3**. Its model files are from a separate repository.
The selected Russian `denis` voice's source dataset is marked **CC0** in its
official model card. Source/model licensing and GPL obligations still require
review before bundling an installer or selling a commercial release.
This branch does **not** redistribute engine binaries or model weights.

- Engine: https://github.com/OHF-Voice/piper1-gpl
- Piper API: https://github.com/OHF-Voice/piper1-gpl/blob/main/docs/API_PYTHON.md
- Selected voice model card: https://huggingface.co/rhasspy/piper-voices/blob/v1.0.0/ru/ru_RU/denis/medium/MODEL_CARD

## Regression gates and unresolved RT0 proof

Required Linux CI executes Piper's loopback auth/model/PCM contract and
offline first-run checksum, download failure, and cache tests. Windows CI
validates launcher safety and also downloads the pinned real Piper voice,
performs authenticated synthesis and checks the resulting PCM WAV contract.

These checks prove local synthesis on a CI Windows runner, **not** audible
D-ID Echo playback, lip synchronization, latency or the full owner/visitor
journey on the user's Windows machine. No RT0 exit is inferred from CI.

RT0 exit still requires real owner + visitor, D-ID Expressive audible audio
and visible moving video, measured first audio latency and A/V drift,
interrupt/STOP and fail-closed revocation, verified cost/usage and human
review on the frozen candidate. Never describe HTTP WAV generation as
confirmed playback by the vendor or visitor.

**Licensing warning:** Irina's RHVoice-derived dataset license is not verified. Irina is for development/interim use only and must be replaced before any commercial release. Denis model card states CC0; the Piper engine has GPL-3.0-or-later obligations that require independent review.
