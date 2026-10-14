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

## Staged Windows handoff and voice rights

Windows Owner Lab builds into an isolated target/rt0-staging Cargo cache,
and runs from alternating target/rt0-live/a and b slots. It validates
the selected provider and warms a real authenticated private speech endpoint
before retiring an inactive old session. Voice ports 8897 through 8927
can coexist during preparation; a foreign listener is never terminated.

Build failures, credential errors and TTS warmup failures preserve the
previous process. The final handoff can still fail after stopping the old
listener; this is not zero-downtime deployment or RT0 proof.

The user prefers actress-reference Sveta. This is evaluation-only and
subject to external audio licensing restrictions. No production voice
cloning or Echo/TtsPort binding is authorized without explicit rights
clearance and human approval.

## Exact-candidate full Owner Lab canary before restart

The Windows restart launcher now starts the fully configured candidate on a
separate unoccupied loopback port after the isolated build, provider credential
probe and authenticated private speech warmup. Before any old listener is
stopped, the canary must start as the exact staged executable, acknowledge
egress and RT0 evidence mode when requested, report text-and-voice provider
readiness, and have no active session. Unknown or competing listeners are
rejected before any HTTP probe. The canary process is terminated through
its retained process handle and its port must be released.

A failed canary startup, readiness check, identity verification or teardown
leaves the old Owner Lab running. This does not replace a human owner/visitor
session, authorize actress-reference cloning, prove D-ID audio/video playback,
or provide automatic rollback after the final port handoff.

Canary readiness also verifies that the embedded HTML entrypoint and
JavaScript application bundle respond successfully. Canary listener teardown
allows a short bounded Windows TCP release delay, but any lingering listener
blocks the primary cutover rather than being ignored.

### Windows real-process CI evidence (not live provider)

The Windows CI also starts two actual Rust Owner Lab processes on separate
loopback ports with isolated, intentionally fake provider credentials,
proves both HTTP endpoints and bundled browser assets, preserves the old
instance after an intentionally invalid-provider canary startup, then
tests a clean primary-port handoff. These checks make no D-ID, STT, LLM
or other external network calls. They do not prove an actual owner/visitor
session or licensed Sveta voice synthesis; those remain manual RT0 gates.

Real-process CI restores a synthetic, structurally owner-reviewed Persona
fixture from an isolated temporary store. This verifies the canonical
conversation_readiness guard instead of weakening it to accept an unreviewed
Persona. No actual owner identity or human approval is inferred from the test.

## User-observed live-session performance and native voice limitations

October 12 owner/visitor evidence on exact SHA ec3591ed6e8e:
- Owner: first useful video ~11.46 s (backend ~8.32 s); visitor: ~6.32 s
  (backend ~4.64 s). This backend stage is external D-ID/session creation,
  not a microphone/STT problem or a guarantee of future startup time.
- STT 247/293 ms; LLM total 4218/4425 ms; browser-observed first audio
  3695/3663 ms from the voice request. Some work overlaps through streaming.
- No A/V timestamp or confirmed canonical playback; do not infer lip sync,
  provider completion or listening satisfaction from a green browser event.

The native Windows SAPI bridge now retains one verified Russian female voice
host for serial speech requests, avoiding a new PowerShell startup on every
clause/resumed sentence. The local process uses fixed code, bounded base64
stdin/stdout IPC, no phrase arguments/logging, and no inherited VPR provider
secrets. Errors and hung inference discard the child and fail closed; next
request can start a fresh local voice. The bounded speaking-rate setting
VPR_WINDOWS_SAPI_RATE defaults to +1 (range -2..3); it alters speed, NOT the
voice identity or native expressiveness. A SAPI voice remains synthetic and
may be perceived as robotic. Real naturalness requires a separately licensed,
consented Russian neural female voice and fresh human audition.

The interrupted-answer UI acknowledges the click synchronously, prevents
duplicate requests, and reports time through server authorization plus
private PCM preparation/delivery. This is NOT time until actual audible Echo
playout; do not label it as such. Production RT0 still requires measured
real owner/visitor playback and human quality acceptance.

The reusable SAPI host receives a positively allowlisted operating-system
environment: no user-supplied provider token, application key, or miscellaneous
process secret is inherited even under non-VPR variable names. Browser E2E
asserts immediate Resume UI acknowledgement, duplicate-click fencing,
denial cleanup and successful canonical suffix replay.

## Owner/visitor Echo server-owned STOP correction — October 12

Observed live owner: video 6526 ms; STT 304 ms; LLM 1262 ms; first audio
2303 ms; browser recovery at 12412 ms; canonical playback not proven.
User reported metallic Windows SAPI voice and unresponsive Interrupt button.

Cause: Echo returns browser interrupt=false because only its private server
sender may issue STOP, but UI wrongly required browser interrupt=true.
Backend had not advertised canonical server-owned Interrupt capability.

Now Echo with private backend advertises canonical Interrupt without granting
the browser a publisher. The button stays usable on active LiveKit Echo
with a server STOP, immediately mutes local audio, prevents duplicate STOP
calls, and waits for epoch-scoped /api/avatar/interrupt confirmation.
Failure revokes session. Local silence is never remote provider STOP proof.

Browser E2E tests real UI click through Rust and hermetic Python Echo worker,
and verifies immediate mute, sender isolation and provider_stop_sent receipt.
Real audible D-ID playout and voice quality remain human RT0 gates.

The browser E2E also simulates an HTTP STOP refusal after a real microphone
turn. It verifies immediate local mute, canonical session revocation, LiveKit
viewer teardown, no browser publisher permissions and private provider STOP
on the revocation path. The refusal is intentionally test-injected; it is
not misrepresented as a real remote D-ID outage.

The STOP-denial E2E injects the failure only after one completed canonical
voice attempt. Startup/greeting interrupt traffic is passed through to the
real server so that a failure test never changes the prerequisite voice journey.

Negative STOP-failure E2E now uses an independent test-only mailbox barrier:
the REAL browser first reports completing its microphone→Rust→Echo answer.
Only after the external Playwright controller observes this event does it arm
an intercepted 503 for the next real user STOP click. Startup/greeting STOPs
pass through unaffected. This avoids manufacturing an early pre-answer
provider outage, and it does not create a privileged production testing route.

The negative browser E2E separates concerns: the already existing positive
journey tests microphone -> STT -> LLM -> private Echo and its ordinary STOP.
For the failure journey an external test controller issues a REAL authorized
Rust /api/avatar/speak while the viewer is connected, verifies private Echo
audio_stream_opened and audio_bytes_written, and only THEN arms an HTTP
failure for the user's next Stop click via an independent fixture mailbox.
This prevents introducing a fake failure during unrelated startup/greeting
traffic and proves provider authority revocation, local mute, browser publisher
isolation and viewer disconnect on a real provider-output session.
