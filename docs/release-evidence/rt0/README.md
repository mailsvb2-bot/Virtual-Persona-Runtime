# RT0 release evidence

This directory is intentionally **not** proof that RT0 is complete.

An RT0 exit candidate must bind evidence to one exact combination of:

- Git commit;
- `RT0_RELEASE_SPEC.md` version + SHA-256;
- Golden manifest SHA-256;
- provider/model/voice/avatar representation versions, a sanitized provider-state manifest, and configuration fingerprints;
- automated unit/contract/E2E results;
- owner and non-owner conversation results;
- Golden Set and permission-suite results;
- latency distributions;
- cost evidence;
- human-evaluation rubric/results;
- known limitations.

Until those artifacts exist for the exact candidate, RT0 remains `EXPERIMENTAL`.

## Golden harness

The deterministic checker lives in `crates/vpr-evaluation`; its minimum manifest is `docs/evaluation/rt0_golden_minimum.json`. The bound evidence contract and CLI are documented in `docs/evaluation/README.md`.

Store real observation inputs outside the repository unless they are explicitly sanitized. A generated report is release evidence only when its candidate, suite digest, ReleaseSpec digest, and recomputed provider-state digest match the same exact candidate and real provider/model/representation state as the rest of the RT0 evidence bundle.

## Exit-evidence gate

The deterministic exit-evidence checker is `vpr-rt0-exit-evidence` in `crates/vpr-evaluation`. It rejects mock/synthetic origins, stale/cross-candidate bindings, failed Golden evidence, missing acceptance paths, quality regressions, unmeasured cost, privacy/permission failures, incomplete human review, and unreviewed limitations.

The Owner Lab browser exposes a CSRF-protected `Скачать evidence snapshot` action only for terminal (`revoked`/`closed`) sessions. The response bytes are downloaded directly without JSON reserialization, and Owner Lab blocks creation of the next session with `EVIDENCE_EXPORT_REQUIRED` until the prior terminal snapshot has been explicitly exported. After a page reload, the server-side gate remains authoritative and the independent export action can still export the terminal snapshot. This prevents the single in-memory recorder from silently discarding an owner snapshot when the operator proceeds to the visitor session; it does not make the snapshot real-provider or human-review evidence by itself.

The private owner-specific Golden report must be generated through `vpr-rt0-owner-golden` from the exact private owner suite/evidence, ReleaseSpec, provider-state bytes and candidate SHA. The command refuses non-owner suites and does not overwrite an existing report.

A checker PASS is necessary evidence hygiene, not proof that the referenced private artifacts are genuine. Keep the private Golden evidence bundle, the recomputed bound Golden report, the exact sanitized provider-state manifest, the sanitized live-provider probe, the sanitized conversation-attempt receipt, the raw sanitized Owner Lab session snapshots, the bound Owner Lab session aggregate, the sanitized exit manifest, and every hashed underlying artifact together for the exact candidate. The inventory preflight schema `rt0-evidence-inventory-0.8` requires the exact `release-spec.md` and `private-golden-evidence.json` bytes plus raw session snapshots themselves, independently binds the ReleaseSpec digest to both the exit manifest and bound Golden report, recomputes the archived bound Golden report from the private Golden bytes with the compiled mandatory suite, validates the live-provider probe with the same canonical semantic validator used by the exit gate, and validates the exit manifest plus supporting-artifact bindings with the same canonical supporting validator used by the release gate. It verifies exact candidate/provider state, presented Golden/probe/conversation/session digests, supporting projections, raw-snapshot recomputation, and owner/visitor conversation-claim derivation from the credentialed receipt plus server-role-bound session snapshots before `inventory_complete` can become true; missing, stale, tampered, duplicate, extra or unbound evidence keeps the inventory incomplete. The release exit CLI also requires the ten canonical supporting evidence files as explicit verifier inputs and hashes their exact bytes against the CI, E2E, owner/visitor conversation, acceptance, quality, cost, privacy/permissions, human-evaluation and known-limitations digests declared in `exit-evidence.json`. For each of the nine JSON files it additionally requires the parsed document to equal the canonical supporting projection with only `artifact_sha256` omitted. CI/E2E projections also carry `candidate_sha`; owner/visitor conversation, acceptance, quality, cost, privacy/permissions, and human-evaluation projections carry both `candidate_sha` and `provider_state_sha256`. A changed claim, stale passed automation result, or real-evidence artifact from another candidate/provider configuration therefore remains invalid even if its new file digest is honestly recomputed and written back into the manifest. `known-limitations.md` remains an exact-byte document binding. It then re-runs the compiled mandatory 13-case Golden suite, requires the recomputed report to exactly match the archived report, recomputes the provider-state digest, verifies the exact live-provider probe plus conversation/session evidence digests and candidate/provider-state bindings, recomputes the session aggregate from the raw snapshots, and rejects provider/model/representation drift or cross-candidate runtime-evidence reuse. `release.rt0_exit_gate` remains `NOT_IMPLEMENTED` until such real evidence exists and is reviewed.
## Local exact-candidate doctor

Before spending provider calls on a real RT0 candidate, the operator can validate the local inputs and provider configuration without network egress:

```text
cargo run -p vpr-live-proof -- doctor \
  /secure/input/probe.raw \
  /secure/input/reviewed-profile.json \
  /secure/input/owner.raw \
  /secure/input/visitor.raw
```

The doctor requires a clean exact Git candidate, absolute private input paths outside the worktree, valid PCM/profile inputs, and a complete provider configuration using the same provider-construction path as Owner Lab/live-proof. It does not require `VPR_LIVE_PROOF_ALLOW_EGRESS=true`, does not call external providers, and writes no evidence artifact. Its stdout receipt is sanitized and explicitly carries `egress_performed=false` and `release_evidence=false`.

A doctor PASS means only that the local candidate, private inputs and provider configuration are structurally ready for the credentialed `candidate` run. It does not prove provider reachability, media playback, quality, privacy acceptance, human review, or RT0 completion.

## Credentialed live-proof preflight

`vpr-live-proof` is the fail-closed preflight for credentialed RT0 evidence. Run it from a clean checkout of the exact candidate and write the sanitized provider-state artifact outside the checkout, for example:

```text
VPR_LIVE_PROOF_ALLOW_EGRESS=true cargo run -p vpr-live-proof -- /secure/evidence/provider-state.json
```

The preflight reuses the same `ProviderBundle` composition path as Owner Lab, requires one explicitly selected supported realtime-avatar provider plus both configured STT and LLM providers, derives the candidate from `git rev-parse HEAD`, and rejects a dirty worktree. D-ID is one supported avatar adapter, not an RT0 requirement; `local-open-source` can be selected without configuring any D-ID credential. Its receipt contains only provider/model/representation descriptors and SHA-256 configuration fingerprints; provider API keys are neither serialized nor included in the fingerprints.

A preflight PASS proves only that an exact clean candidate has a complete local provider configuration and explicit egress authorization. It does **not** prove external provider reachability, conversation quality, or RT0 completion. `provider.real_*` and `release.rt0_exit_gate` remain `NOT_IMPLEMENTED` until credentialed live runs and the rest of the exit evidence exist.

Live-proof preflight output must use an absolute path outside the canonical Git worktree; evidence generation must not dirty the candidate it claims to describe.

### DeepSeek LLM configuration

Owner Lab and `vpr-live-proof` accept `VPR_OWNER_LAB_LLM_PROVIDER=deepseek` through the existing OpenAI-compatible transport while preserving `deepseek` as the runtime and sanitized evidence provider identity. The endpoint variable is the full Chat Completions endpoint used by the adapter, not an SDK base URL.

For the current DeepSeek public API:

```text
VPR_OWNER_LAB_LLM_PROVIDER=deepseek
VPR_OWNER_LAB_LLM_ENDPOINT=https://api.deepseek.com/chat/completions
VPR_OWNER_LAB_LLM_MODEL=deepseek-flash
VPR_OWNER_LAB_LLM_API_KEY=<secret set outside the repository>
```

Provider keys must remain outside the repository and are never serialized into provider-state or live-proof receipts. If a different DeepSeek model or endpoint is used, that exact model/endpoint participates in the configuration fingerprint and therefore produces a different evidence-bound provider state.

For the realtime Owner Lab path, the DeepSeek adapter explicitly sends both `reasoning_effort="none"` and `thinking.type="disabled"`, plus the bounded spoken-response token cap. The realtime tuning is part of the sanitized configuration fingerprint so evidence cannot silently mix a thinking-enabled run with the low-latency profile.

## Credentialed live-provider probe

After preflight, `vpr-live-proof probe` can test real provider reachability for the exact clean candidate:

```text
VPR_LIVE_PROOF_ALLOW_EGRESS=true cargo run -p vpr-live-proof -- probe /secure/input/utterance.pcm /secure/evidence/provider-state.json /secure/evidence/provider-probe.json
```

The input must be raw PCM S16LE, mono, 16 kHz and must live outside the Git worktree. STT and LLM execute only through canonical `ActiveTurn::execute_stt` / `execute_llm`; the realtime avatar control plane is opened and closed through `OwnerLabEngine`. The transcript is not forwarded to the LLM: the LLM reachability probe uses a fixed safe Russian prompt. A standalone TTS provider is **not required** when the selected realtime-avatar provider performs speech synthesis as part of its browser-connected text-to-spoken-avatar path.

The serialized probe receipt schema is `rt0-live-provider-probe-0.3`. It contains only stage latency, usage/cost counters, transcript/output character counts, exact candidate/provider-state binding, SHA-256 + duration binding for the private STT PCM input, and realtime-avatar control-plane open/close timing. It never stores raw input audio, transcript text, generated reply, WebRTC signaling, provider session identifiers, credentials, or provider-generated media. The headless probe intentionally does **not** submit speech or claim audible playback: RT0 media proof is browser-bound and the selected realtime-avatar adapter may require browser WebRTC negotiation before media can be observed. Actual voice evidence comes from the browser Owner Lab session snapshot: canonical output binding plus observed remote audio, rendered video, A/V-sync samples, interruption evidence and human voice review on the same exact candidate/provider state.

A probe PASS proves credentialed provider reachability only. It does not prove a real owner/non-owner conversation, rendered media delivery, human quality, Golden Set completion, or RT0 release readiness. `provider.real_*` and `release.rt0_exit_gate` therefore remain `NOT_IMPLEMENTED` until the required external evidence exists and is reviewed.
## Credentialed owner/visitor conversation attempt

`vpr-live-proof conversation` reuses the canonical Owner Lab engine for one owner turn followed by one visitor-scoped turn over the same reviewed `DIGITAL_TWIN` Persona and the same credentialed STT/LLM/avatar provider composition:

```text
VPR_LIVE_PROOF_ALLOW_EGRESS=true cargo run -p vpr-live-proof -- conversation \
  /secure/input/reviewed-profile.json \
  /secure/input/owner-utterance.raw \
  /secure/input/visitor-utterance.raw \
  /secure/evidence/provider-state.json \
  /secure/evidence/conversation-attempt.json
```

The profile and both raw PCM S16LE/mono/16 kHz inputs must be absolute files outside the Git worktree. The profile schema is `rt0-live-conversation-profile-0.1`; it contains a private Persona id plus direct owner claims, requires explicit `owner_review_confirmed=true` and per-claim `owner_approved=true`, and is reconstructed through the canonical capture/review lifecycle before any session starts. All input and output paths must be distinct.

The sanitized receipt schema is `rt0-live-conversation-attempt-0.1`. It binds the exact candidate and sanitized provider-state digest, hashes the private profile, Persona id, both audio inputs, transcripts and replies, and records character counts, locale, server-stage latency and complete-known cost totals. It never serializes owner claims, transcript/reply text, raw audio, WebRTC signaling, provider session identifiers or credentials.

A successful attempt proves only that real credentialed provider calls traversed the canonical owner and visitor policy paths and that generated output was submitted to the realtime-avatar provider. It deliberately records `browser_media_playback="not_proven"`, `video_render="not_proven"`, and `human_review="not_proven"`. It therefore cannot by itself satisfy the RT0 real-conversation, media-plane, A/V-sync, privacy acceptance or human-evaluation exit conditions.

## Credentialed exact-candidate capture

For a new RT0 exact-candidate headless capture, prefer the single-file `candidate-bundle` mode:

```text
VPR_LIVE_PROOF_ALLOW_EGRESS=true cargo run -p vpr-live-proof -- candidate-bundle \
  /secure/input/probe.raw \
  /secure/input/reviewed-profile.json \
  /secure/input/owner.raw \
  /secure/input/visitor.raw \
  /secure/evidence/candidate-bundle.json
```

The bundle schema is `rt0-live-proof-candidate-bundle-0.1`. It contains the sanitized provider
state, live-provider probe and owner/visitor conversation-attempt receipt bound to one exact
candidate and one provider-state digest. The command validates all private inputs and the immutable
destination before any provider egress, requires the provider composition to remain unchanged
between probe and conversation, re-verifies the clean Git candidate, serializes the whole capture,
and publishes that one file with create-new semantics. An existing artifact is never overwritten.

Downstream RT0 evidence tools still consume the three canonical component artifacts. Do not
copy/paste or hand-edit nested JSON out of the bundle. Extract them through the fail-closed canonical
projection while the checkout is still on the exact candidate:

```text
cargo run -p vpr-live-proof -- candidate-bundle-extract \
  /secure/evidence/candidate-bundle.json \
  /secure/evidence/provider-state.json \
  /secure/evidence/provider-probe.json \
  /secure/evidence/conversation-attempt.json
```

Extraction performs no provider egress. It requires a clean checkout of the exact candidate encoded
by the bundle, validates the bundle schema, recomputes and verifies the provider-state digest,
validates the provider probe and sanitized owner/visitor conversation receipt, rejects output paths
inside the worktree, refuses overwrite, and only then publishes the three canonical downstream
artifacts. If any write or final candidate check fails, already-created projection files are removed.

The older `candidate` mode remains available when three separate files are required directly during
credentialed capture:

```text
VPR_LIVE_PROOF_ALLOW_EGRESS=true cargo run -p vpr-live-proof -- candidate \
  /secure/input/probe.raw \
  /secure/input/reviewed-profile.json \
  /secure/input/owner.raw \
  /secure/input/visitor.raw \
  /secure/evidence/provider-state.json \
  /secure/evidence/provider-probe.json \
  /secure/evidence/conversation-attempt.json
```

Both capture modes freeze the candidate and reject provider-state drift. The shared artifact writer now uses
a same-directory temporary file plus an atomic create-new hard-link publication, so a destination
that appears while provider work is running cannot be silently overwritten on platforms where
rename would otherwise replace it.

Neither mode proves browser media playback, rendered video, A/V sync, privacy acceptance, Golden
completion, human quality, or RT0 exit readiness. Browser Owner Lab evidence and the remaining
supporting artifacts are still mandatory.

## Owner Lab live-session evidence

Owner Lab can collect one in-memory sanitized session-evidence snapshot for the current canonical session. Browser voice requests carry a local evidence request sequence; the backend correlates it with the canonical turn sequence and records only stage latency, usage/cost counters, stable failure codes, and browser-observed media-plane latency events. Raw microphone PCM, transcript/reply text, SDP/ICE, provider stream/session identifiers, and credentials are not part of this artifact.

The browser observes first rendered video through `requestVideoFrameCallback`, remote speech onset through an `AnalyserNode` RMS detector, interruption stop through sustained post-interrupt silence, and reconnect restoration through WebRTC connection-state transitions. The browser never calls runtime delivery APIs directly. For `audio_started`, the backend first resolves the completed request to its runtime-issued canonical turn and output-segment sequence, reconciles that exact `OutputDeliveryHandle` to runtime `Played`, and only then records the sanitized media event and per-attempt playback confirmation. Stale, duplicate, cross-turn, cross-output and post-session acknowledgements fail closed. This proves canonical playback for that exact voice output; it still does not prove A/V sync, owner/non-owner conversation quality, or RT0 completion.

## Owner Lab session evidence aggregate

Owner Lab emits sanitized `rt0-owner-lab-session-evidence-0.6` snapshots using the shared contract in `vpr-evaluation`. The server records `participant_role=owner|visitor` when the canonical session starts, so evidence consumers never infer role from filenames or client claims. `vpr-rt0-session-aggregate` can combine one or more of those snapshots into deterministic latency/cost evidence without re-reading transcript, reply, raw PCM, SDP, or provider session identifiers.

The aggregate can support browser-observed first-audio, interruption-stop, first-rendered-video, reconnect, STT latency, full LLM latency, voice LLM first-meaningful-response latency, server-stage latency, complete cost totals, runtime-backed canonical playback, and request-scoped A/V sync absolute-offset samples when those fields are actually present. Schema `0.6` derives `canonical_playback_proven` from per-attempt turn/output bindings plus runtime-confirmed playback, records both full voice LLM latency and first meaningful LLM response latency for bottleneck diagnosis, and derives `av_sync_proven` only when every completed request also has all three sequence-numbered `web_rtc_estimated_playout_timestamp` samples after that playback confirmation. The browser samples the W3C WebRTC Stats estimated playout timestamps for the single active inbound audio/video pair; unavailable, packetless or ambiguous stats produce no sample rather than a fallback estimate. The exit checker recomputes the aggregate from raw snapshots and requires its exact A/V-sync distribution to equal `QualityEvidence.av_sync_absolute_offset` before evaluating the `p95 <= 120 ms` target. This closes the A/V-sync evidence-integrity bridge but does not complete RT0.

### Windows secure provider profile

For the local Windows operator path, run `cargo run -p vpr-owner-lab --bin vpr-provider-credentials -- set` once and select the avatar provider. The secure profile can be created directly for D-ID or for `local-open-source`; a local-avatar profile does not require or invent D-ID credentials. If the operator still has a CMD session containing `VPR_*` values, `cargo run -p vpr-owner-lab --bin vpr-provider-credentials -- import-env` imports the selected avatar configuration plus the configured STT/LLM credentials without displaying or retyping secrets. Historical profiles that predate avatar selection continue to deserialize as D-ID for migration compatibility only.

`ProviderBundle` resolves explicit process environment first and then the matching Windows credential profile. A stored secret therefore cannot silently override an explicitly selected different provider. For hermetic validation, `VPR_PROVIDER_CREDENTIAL_SOURCE=environment` disables Windows Credential Manager fallback for that process and makes incomplete environment configuration fail closed. The normal fallback remains available to Owner Lab and `vpr-live-proof`, while provider-state and evidence remain secret-free.

For an interactive Owner Lab evidence session, prefer the explicit per-process opt-in:

```powershell
cargo run -p vpr-owner-lab -- --allow-egress
```

The flag is intentionally not persisted in Windows Credential Manager. A new process is fail-closed again unless the operator explicitly supplies `--allow-egress` (or the existing `VPR_OWNER_LAB_ALLOW_EGRESS=true` automation override).
