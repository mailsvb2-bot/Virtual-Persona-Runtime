# VoiceProfile (experimental RT0)

Profiles are non-secret local JSON, kept outside Git, e.g. `%LOCALAPPDATA%\\VPR\\voice-profiles\\interim.json`. An example is available under `docs/examples/`. Schema `vpr-voice-profile-0.1` contains `profile_id`, integer `version`, `status` (`INTERIM`, `CANDIDATE`, `ACCEPTED`), TTS provider/model/voice, and optional `rights_ref_sha256`. For `ACCEPTED`, SHA-256 of an external `voice-rights.json` is mandatory. Never put API keys, tokens, endpoints containing credentials or real person voice assets into profile or version control.

Set `VPR_OWNER_LAB_VOICE_PROFILE_PATH` to the absolute path. Supply exactly matching `VPR_OWNER_LAB_TTS_PROVIDER`, `VPR_OWNER_LAB_TTS_MODEL`, and `VPR_OWNER_LAB_TTS_VOICE`; use `VPR_OWNER_LAB_TTS_ENDPOINT` and `VPR_OWNER_LAB_TTS_API_KEY` for transport/authentication only. Conflicts fail closed. Qwen preset selection uses `metrotrance-qwen-preset` with `Serena` or `Ryan` as configured speaker; configure the local bridge consistently using `VPR_QWEN_PRESET_SPEAKER`.

In strict RT0 mode, a configured standalone TTS requires a profile; `CANDIDATE` and alternative fallback are forbidden. Switching to a new voice is explicit: create new file, increment version, restart, and repeat evidence collection since provider-state SHA changes. Integrated-speech avatar with no standalone TTS remains possible. `INTERIM` is not proof of audible quality; the owner's voice quality review is required for RT1 per Amendment A1. Licensing must be verified independently; examples are not rights evidence.

## Pinned interim Piper identities

| VoiceProfile | Voice | Licensing |
|---|---|---|
| `interim-piper-irina` | `ru_RU-irina-medium` | Dataset license unknown (RHVoice). Development only; replace before release |
| `interim-piper-denis` | `ru_RU-denis-medium` | Model-card dataset CC0; Piper engine GPL license review required |

Both use `provider=piper`, `model=piper`, and `status=INTERIM`. These voices can never be marked `ACCEPTED` by the current RT0 parser. `scripts/windows-owner-lab-restart.ps1 -Piper` automatically materializes the selected example to LocalAppData before the strict evidence gate, or accepts a compatible explicit `-VoiceProfile`. The locally loaded ONNX model must match the declared voice. No commercial voice license is implied.
