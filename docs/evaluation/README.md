# RT0 Golden evaluation harness

`vpr-evaluation` is a deterministic evidence checker. It is not a Persona runtime, provider orchestrator, or release authority.

The minimum synthetic manifest is `docs/evaluation/rt0_golden_minimum.json`. It covers Russian text handling plus the RT0 attribution, privacy, revocation, cancellation, and identity-stability minimum from the ReleaseSpec.

## Bound evidence artifacts

The evaluator binds five inputs to one report:

1. Golden suite JSON;
2. sensitive observation/evidence JSON;
3. exact `RT0_RELEASE_SPEC.md`;
4. sanitized provider-state JSON;
5. exact Git candidate SHA supplied on the command line.

The evidence JSON has `binding` and `observations`. Binding schema `rt0-evidence-binding-0.1` contains the candidate SHA plus SHA-256 digests of the suite, ReleaseSpec, and provider-state file. The evaluator recomputes all three artifact digests itself and rejects stale or mismatched evidence.

Golden suite schema is fixed at `rt0-golden-0.1`; unknown schema versions or unknown fields fail closed. Provider-state schema `rt0-provider-state-0.1` requires exactly one STT, LLM, and Avatar descriptor. Each descriptor names its provider and model/representation and includes a lowercase SHA-256 fingerprint of its deterministic sanitized configuration. `rt0_provider_state.synthetic.example.json` demonstrates the shape only; it is not release evidence.

Provider fingerprints must never be calculated from or expose API keys, authorization headers, private documents, raw audio/video, prompts, or other secrets. Archive the sanitized configuration artifact used to produce each fingerprint with the private release evidence so the fingerprint is reproducible.

`observations` may contain raw test responses while evaluation is running. Treat the evidence input as sensitive. The generated report includes SHA-256 of the exact sensitive evidence input and intentionally emits only binding/provider metadata, case IDs, pass/fail state, and stable failure codes; it does not echo prompts, response text, or private sentinels.

## Run

```bash
cargo run -p vpr-evaluation -- \
  docs/evaluation/rt0_golden_minimum.json \
  /secure/path/rt0-evidence.json \
  docs/releases/RT0_RELEASE_SPEC.md \
  /secure/path/rt0-provider-state.json \
  "$(git rev-parse HEAD)"
```

Exit codes are stable: `0` means the bound Golden bundle passed, `1` means the binding is valid but one or more Golden cases failed, and `2` means input or binding is invalid/stale.

A successful synthetic or mock run is **not** RT0 exit evidence. Release evidence still requires the same exact candidate/provider state to have real owner and non-owner conversations, measured latency/cost, permission/privacy results, and human evaluation as required by the Canon and ReleaseSpec.

## RT0 evidence inventory preflight

`vpr-rt0-evidence-inventory` is a non-promoting filesystem preflight for an external RT0 evidence directory. It hashes the expected evidence artifacts and checks the exact candidate/provider-state bindings exposed by the bound Golden report, live-provider probe, credentialed owner/visitor conversation-attempt receipt, and bound Owner Lab session aggregate. It does not evaluate conversation quality, human review, latency thresholds, privacy outcomes, or release readiness.

Use canonical filenames in the private evidence directory and run:

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-evidence-inventory -- \
  /secure/evidence/rt0-candidate \
  "$(git rev-parse HEAD)"
```

Exit `0` means only that every expected inventory slot is present and the candidate/provider-state bindings checked by this preflight match. The canonical inventory now requires `conversation-attempt.json` and `bound-session-aggregate.json`; malformed, stale, cross-candidate, or cross-provider bindings keep the inventory incomplete. The JSON field is deliberately named `inventory_complete`; the tool never emits a `ready` claim. Exit `1` means files are missing or core bindings do not match. Exit `2` is command/input failure. A complete inventory must still pass `vpr-rt0-exit-evidence` and the underlying artifacts must genuinely represent the real evidence they claim.

## RT0 exit-evidence gate

`vpr-rt0-exit-evidence` checks whether one sanitized release-evidence manifest is complete and bound to the same exact candidate as a successful bound Golden report. The exit checker also re-evaluates the archived private Golden evidence against the compiled mandatory RT0 minimum suite before trusting that report. It does not create evidence and cannot turn mock/synthetic results into real evidence.

The gate requires real owner and visitor Russian voice/video conversations, owner interruption, the ReleaseSpec acceptance matrix, measured QualityContract latency distributions, measured cost, zero accepted private-context leakage and false owner attribution, revocation/egress-denial proof, all five mandatory human-review dimensions, an explicit usability decision, and reviewed known limitations. Thresholds are taken unchanged from `RT0_RELEASE_SPEC.md`.

Run it with:

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-exit-evidence -- \
  /secure/path/rt0-exit-evidence.json \
  /secure/path/bound-golden-report.json \
  /secure/path/private-golden-evidence.json \
  /secure/path/rt0-provider-state.json \
  /secure/path/live-provider-probe.json \
  /secure/path/conversation-attempt.json \
  /secure/path/bound-session-aggregate.json \
  docs/releases/RT0_RELEASE_SPEC.md \
  "$(git rev-parse HEAD)"
```

The exit checker recomputes the sanitized provider-state digest from the separate provider-state file and requires its parsed contents to exactly match the provider state embedded in the Golden report. It also recomputes the SHA-256 of the separate credentialed live-provider probe, credentialed owner/visitor conversation-attempt receipt, and bound Owner Lab session aggregate, and requires all three artifacts to be bound to the same exact candidate and provider state. The session aggregate is structurally checked as non-promoting browser-observed evidence and cannot claim canonical playback or A/V sync. A valid rehashed probe from another commit or provider configuration is rejected structurally. It also re-runs the bound Golden evaluator over the private Golden evidence bundle using the compiled mandatory `rt0_golden_minimum.json` suite, the exact ReleaseSpec bytes, provider state, and candidate SHA; the recomputed report must exactly match the archived sanitized Golden report. This prevents shortened/forged Golden reports and cross-provider/model/representation reuse.

Exit codes are stable: `0` means the supplied exact-candidate evidence satisfies the deterministic gate, `1` means evidence is structurally valid but one or more mandatory RT0 conditions fail, and `2` means evidence is malformed, stale, tampered, or cross-candidate. `docs/evaluation/rt0_exit_evidence.synthetic.example.json` is schema documentation only; every substantive origin in it is `synthetic`, so it is intentionally ineligible for RT0 exit.

Even a `0` from this experimental checker does not by itself promote `release.rt0_exit_gate`: the archived evidence artifacts must actually exist and correspond to the real provider/model/representation state named by the bound Golden report.

## Owner Lab session-evidence aggregation and binding

`vpr-rt0-session-aggregate` consumes one or more sanitized `rt0-owner-lab-session-evidence-0.2` JSON snapshots and deterministically derives the latency distributions that those snapshots can actually support: STT, LLM, avatar-submit, server-total, browser-observed first audio, interruption stop, first rendered video, and reconnect restoration. It also sums estimated/provider cost only when every completed provider stage contains that cost field; partial cost never becomes a fake complete total.

The legacy aggregation mode remains available:

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-session-aggregate -- \
  /secure/evidence/session-1.json /secure/evidence/session-2.json
```

For release-evidence preparation, use `bind` mode with the exact sanitized provider-state artifact and exact candidate SHA:

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-session-aggregate -- \
  bind \
  /secure/evidence/rt0-provider-state.json \
  "$(git rev-parse HEAD)" \
  /secure/evidence/session-1.json /secure/evidence/session-2.json
```

The bound receipt schema is `rt0-owner-lab-session-aggregate-binding-0.2`. It includes the exact candidate SHA, SHA-256 of the exact provider-state bytes, SHA-256 of every raw snapshot artifact in input order, and the deterministic aggregate. Provider-state structure is revalidated and duplicate/malformed snapshot artifacts fail closed.

Binding does not let browser observations self-promote. In schema `0.2`, a completed voice attempt carries the runtime-issued canonical turn and output-segment sequences; `audio_started` can mark that attempt playback-confirmed only after the backend reconciles the exact `OutputDeliveryHandle` to runtime `Played`. The aggregate sets `canonical_playback_proven=true` only when every completed attempt in every input session has that exact per-attempt confirmation. `av_sync_proven` remains false. The current RT0 exit checker still rejects a playback-promoting bound aggregate because it does not yet receive and recompute the raw session snapshots itself; this artifact is therefore exact-candidate evidence preparation, not an exit-gate promotion.
