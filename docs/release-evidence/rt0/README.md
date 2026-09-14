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

A checker PASS is necessary evidence hygiene, not proof that the referenced private artifacts are genuine. Keep the private Golden evidence bundle, the recomputed bound Golden report, the exact sanitized provider-state manifest, the sanitized live-provider probe, the sanitized exit manifest, and every hashed underlying artifact together for the exact candidate. The exit checker re-runs the compiled mandatory 12-case Golden suite, requires the recomputed report to exactly match the archived report, recomputes the provider-state digest, verifies the exact live-provider probe digest/candidate/provider-state binding, and rejects provider/model/representation drift or cross-candidate probe reuse. `release.rt0_exit_gate` remains `NOT_IMPLEMENTED` until such real evidence exists and is reviewed.
## Credentialed live-proof preflight

`vpr-live-proof` is the fail-closed preflight for credentialed RT0 evidence. Run it from a clean checkout of the exact candidate and write the sanitized provider-state artifact outside the checkout, for example:

```text
VPR_LIVE_PROOF_ALLOW_EGRESS=true cargo run -p vpr-live-proof -- /secure/evidence/provider-state.json
```

The preflight reuses the same `ProviderBundle` composition path as Owner Lab, requires D-ID plus both configured STT and LLM providers, derives the candidate from `git rev-parse HEAD`, and rejects a dirty worktree. Its receipt contains only provider/model/representation descriptors and SHA-256 configuration fingerprints; provider API keys are neither serialized nor included in the fingerprints.

A preflight PASS proves only that an exact clean candidate has a complete local provider configuration and explicit egress authorization. It does **not** prove external provider reachability, conversation quality, or RT0 completion. `provider.real_*` and `release.rt0_exit_gate` remain `NOT_IMPLEMENTED` until credentialed live runs and the rest of the exit evidence exist.

Live-proof preflight output must use an absolute path outside the canonical Git worktree; evidence generation must not dirty the candidate it claims to describe.

## Credentialed live-provider probe

After preflight, `vpr-live-proof probe` can test real provider reachability for the exact clean candidate:

```text
VPR_LIVE_PROOF_ALLOW_EGRESS=true cargo run -p vpr-live-proof -- probe /secure/input/utterance.pcm /secure/evidence/provider-state.json /secure/evidence/provider-probe.json
```

The input must be raw PCM S16LE, mono, 16 kHz and must live outside the Git worktree. STT and LLM execute only through canonical `ActiveTurn::execute_stt` / `execute_llm`; realtime avatar open/close executes through `OwnerLabEngine`. The transcript is not forwarded to the LLM: the LLM reachability probe uses a fixed safe Russian prompt.

The serialized probe receipt contains only stage latency, usage/cost counters, transcript/output character counts, exact candidate/provider-state binding, SHA-256 + duration binding for the private PCM input, and explicit `conversation_evidence=false` / `output_delivery_proven=false`. It never stores raw audio, transcript text, generated reply, WebRTC signaling, provider session identifiers, or credentials.

A probe PASS proves credentialed provider reachability only. It does not prove a real owner/non-owner conversation, rendered media delivery, human quality, Golden Set completion, or RT0 release readiness. `provider.real_*` and `release.rt0_exit_gate` therefore remain `NOT_IMPLEMENTED` until the required external evidence exists and is reviewed.
