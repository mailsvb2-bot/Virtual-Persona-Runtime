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

`vpr-rt0-evidence-inventory` is a non-promoting filesystem preflight for an external RT0 evidence directory. Schema `rt0-evidence-inventory-0.3` hashes the expected evidence artifacts, parses the canonical `exit-evidence.json`, checks its exact candidate/provider-state and top-level artifact-digest bindings, and reuses the same canonical supporting-artifact validator as the release exit gate for CI/E2E plus the seven real supporting JSON claims and `known-limitations.md`. It also checks the exact candidate/provider-state bindings exposed by the bound Golden report, live-provider probe, credentialed owner/visitor conversation-attempt receipt, and bound Owner Lab session aggregate. Raw sanitized `rt0-owner-lab-session-evidence-0.3` JSON snapshots are discovered in the evidence directory, matched by exact byte digest to the ordered `snapshot_sha256` list in the bound aggregate, and recomputed into the canonical session binding. Missing, tampered, duplicate, stale, cross-candidate/provider, detached supporting claims, or extra/unbound session snapshots keep the inventory incomplete. Snapshot filenames are not trusted as evidence identity. It does not evaluate conversation quality, human review, latency thresholds, privacy outcomes, or release readiness.

Use canonical filenames in the private evidence directory and run:

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-evidence-inventory -- \
  /secure/evidence/rt0-candidate \
  "$(git rev-parse HEAD)"
```

Exit `0` means only that every expected inventory slot is present, `exit-evidence.json` is structurally bound to the presented core/supporting artifacts for the exact candidate/provider state, the external runtime receipts agree with those bindings, and the raw session snapshots exactly reproduce the archived bound session aggregate. The canonical inventory requires `conversation-attempt.json`, `bound-session-aggregate.json`, all ten supporting-evidence files, and every raw session snapshot named by that aggregate's `snapshot_sha256` list; malformed, stale, cross-candidate, cross-provider, detached, missing, tampered, duplicate, or unbound evidence keeps the inventory incomplete. The JSON field is deliberately named `inventory_complete`; the tool never emits a `ready` claim. Exit `1` means files are missing or core bindings do not match. Exit `2` is command/input failure. A complete inventory must still pass `vpr-rt0-exit-evidence` and the underlying artifacts must genuinely represent the real evidence they claim.

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
  /secure/path/rt0-supporting-evidence \
  /secure/path/session-owner.json \
  /secure/path/session-visitor.json \
  docs/releases/RT0_RELEASE_SPEC.md \
  "$(git rev-parse HEAD)"
```

The exit checker recomputes the sanitized provider-state digest from the separate provider-state file and requires its parsed contents to exactly match the provider state embedded in the Golden report. It also recomputes the SHA-256 of the separate credentialed live-provider probe, credentialed owner/visitor conversation-attempt receipt, and bound Owner Lab session aggregate, and requires all three artifacts to be bound to the same exact candidate and provider state. The release CLI additionally requires the canonical supporting-evidence directory containing `ci-evidence.json`, `e2e-evidence.json`, `owner-conversation.json`, `visitor-conversation.json`, `acceptance.json`, `quality.json`, `cost.json`, `privacy-permissions.json`, `human-evaluation.json`, and `known-limitations.md`. Its verified entry point hashes the exact bytes of all ten files and requires each digest to equal the corresponding digest declared inside `exit-evidence.json`. For the nine JSON supporting artifacts it then parses those same bytes and requires exact equality with the canonical supporting projection. Every projection omits only `artifact_sha256`; CI/E2E additionally carry `candidate_sha`, while owner/visitor conversation, acceptance, quality, cost, privacy/permissions, and human-evaluation projections carry both `candidate_sha` and `provider_state_sha256`. Reusing passed automation from another commit or real evidence from another candidate/provider state therefore fails structurally even after an honest digest rebind. `known-limitations.md` remains a reviewed document bound by its exact byte digest because its manifest projection is review status plus document hash rather than JSON claim content. One or more raw sanitized Owner Lab session snapshots are mandatory verifier inputs: the gate recomputes the bound aggregate from those exact bytes, candidate and provider state, then requires exact equality with the archived bound aggregate. Runtime-backed `canonical_playback_proven=true` and A/V-sync proof are accepted only through that recomputation. For A/V sync, the recomputed aggregate must prove all completed playback turns with the required three request-scoped samples, and its exact `av_sync_absolute_offset` distribution must equal `QualityEvidence.av_sync_absolute_offset`; a detached or hand-edited quality distribution is rejected structurally. A valid rehashed probe from another commit or provider configuration is rejected structurally. It also re-runs the bound Golden evaluator over the private Golden evidence bundle using the compiled mandatory `rt0_golden_minimum.json` suite, the exact ReleaseSpec bytes, provider state, and candidate SHA; the recomputed report must exactly match the archived sanitized Golden report. This prevents shortened/forged Golden reports and cross-provider/model/representation reuse.

Exit codes are stable: `0` means the supplied exact-candidate evidence satisfies the deterministic gate, `1` means evidence is structurally valid but one or more mandatory RT0 conditions fail, and `2` means evidence is malformed, stale, tampered, or cross-candidate. `docs/evaluation/rt0_exit_evidence.synthetic.example.json` is schema documentation only; every substantive origin in it is `synthetic`, so it is intentionally ineligible for RT0 exit.

Even a `0` from this experimental checker does not by itself promote `release.rt0_exit_gate`: the archived evidence artifacts must actually exist and correspond to the real provider/model/representation state named by the bound Golden report.

## Owner Lab session-evidence aggregation and binding

`vpr-rt0-session-aggregate` consumes one or more sanitized `rt0-owner-lab-session-evidence-0.3` JSON snapshots and deterministically derives the latency distributions that those snapshots can actually support: STT, LLM, avatar-submit, server-total, browser-observed first audio, interruption stop, first rendered video, reconnect restoration, and request-scoped A/V sync absolute offset when WebRTC estimated playout timestamps are available. It also sums estimated/provider cost only when every completed provider stage contains that cost field; partial cost never becomes a fake complete total.

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

The bound receipt schema is `rt0-owner-lab-session-aggregate-binding-0.3`. It includes the exact candidate SHA, SHA-256 of the exact provider-state bytes, SHA-256 of every raw snapshot artifact in input order, and the deterministic aggregate. Provider-state structure is revalidated and duplicate/malformed snapshot artifacts fail closed.

Binding does not let browser observations self-promote. In schema `0.3`, a completed voice attempt carries the runtime-issued canonical turn and output-segment sequences; `audio_started` can mark that attempt playback-confirmed only after the backend reconciles the exact `OutputDeliveryHandle` to runtime `Played`. Request-scoped A/V sync samples use the explicit `web_rtc_estimated_playout_timestamp` reference and are accepted only after that same request has canonical playback confirmation. The aggregate sets `canonical_playback_proven=true` only when every completed attempt has playback proof, and `av_sync_proven=true` only when every completed attempt also has all three valid sequence-numbered A/V sync samples. The RT0 exit checker recomputes this aggregate from the raw snapshots and requires its exact A/V-sync distribution to equal the QualityEvidence distribution before the normal `p95 <= 120 ms` threshold is evaluated. This closes the A/V-sync evidence-integrity chain; it does not by itself complete RT0 because real owner/visitor, privacy, cost, Golden and human-review evidence remain separately mandatory.
