# RT0 native female Russian voice on Windows

This optional developer RT0 candidate uses the Russian **Microsoft Irina Desktop**
female SAPI voice already installed with Windows. It corrects the gender mismatch
reported in a live D-ID Expressive session; audible naturalness is to be judged
by the user, not inferred from protocol tests.

**Double-click `START_VPR_WITH_VOICE.cmd` in a clean Git clone** of the
selected candidate. This delegates to `scripts/start-rt0-sapi.cmd` and the
unchanged canonical strict-evidence Windows Owner Lab launcher.

Prerequisites: a configured Windows D-ID / Deepgram / DeepSeek secure profile,
Firefox 142+, Python 3.11 (`py -3.11`) and the existing Rust toolchain.
The Windows voice is not bundled, uploaded or downloaded; it must already be
installed under the exact name `Microsoft Irina Desktop`, language `ru-RU`,
gender Female.

One-click path:

- On first use install the exact LiveKit 1.1.19 Echo PCM SDK into an **isolated**
  private virtual environment under
  `%LOCALAPPDATA%\Virtual-Persona-Runtime\Sapi\venv`.
  No Piper model or Piper engine is installed or required in this mode.
- Generate a fresh 32-byte random secret. Start the Python/PowerShell WAV
  bridge on loopback `127.0.0.1:8897`. Before sending the secret, check the
  listener belongs to the exact spawned virtualenv Python interpreter or its
  verified direct Windows redirector child. Reject foreign listeners.
- Test authenticated synthesis and PCM format, validate D-ID Expressive Echo
  readiness and launch the real Owner Lab only after safe preflight.
- Keep text, instructions and provider credentials on the canonical server-owned
  route. The browser is not allowed to issue `did.speak`; it receives media.
  STOP/revoke fences all subsequent delivery. No transcript, token or synthesized
  audio is written to a diagnostic file.

To opt into the older male Piper voice instead, launch
`scripts/start-rt0-piper.cmd`. The `-Piper` and `-SystemVoice` switches
are mutually exclusive.

## Release gates

A 200 WAV response, passing CI and a displayed video track are not proof
of audible synchronization. On the user's Windows device, validate owner and
visitor sessions, natural voice, greeting after first video frame, microphone
selection and hot-switch, STOP → resume through canonical PCM, first-audio and
end-to-end video latency, A/V playout timestamps, costs and human review.
Keep RT0 exit unconfirmed while any are missing.

This mode invokes **Windows-provided** voice files in place. It does not
redistribute Microsoft's voice assets; review Windows voice usage terms before
commercial release. Future cross-platform releases will need another
properly licensed female voice provider.
