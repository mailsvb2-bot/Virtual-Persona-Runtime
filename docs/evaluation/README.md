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

For browser-collected Owner Lab evidence, terminate or revoke each owner/visitor session and use `Скачать evidence snapshot` before starting the next session. Owner Lab returns the sanitized snapshot as exact response bytes and blocks the next session with `EVIDENCE_EXPORT_REQUIRED` until that export request succeeds; after reload, the server-side gate still blocks a new session and the independent export action remains available. Keep both downloaded snapshot files with the exact candidate evidence directory consumed by the inventory and exit tools.

`vpr-rt0-evidence-inventory` is a non-promoting filesystem preflight for an external RT0 evidence directory. Schema `rt0-evidence-inventory-0.7` hashes the expected evidence artifacts, parses the canonical `exit-evidence.json`, checks its exact candidate/provider-state and top-level artifact-digest bindings, binds the exact `release-spec.md` bytes independently to both `exit-evidence.json` and `bound-golden-report.json`, recomputes the archived bound Golden report from the exact `private-golden-evidence.json` bytes using the compiled mandatory RT0 Golden suite, and applies the same canonical semantic validator used by the exit gate to `provider-probe.json`, and reuses the same canonical supporting-artifact validator as the release exit gate for CI/E2E plus the seven real supporting JSON claims and `known-limitations.md`. It also checks the exact candidate/provider-state bindings exposed by the bound Golden report, live-provider probe, credentialed owner/visitor conversation-attempt receipt, and bound Owner Lab session aggregate. The live-provider probe schema `rt0-live-provider-probe-0.3` independently carries sanitized credentialed reachability evidence for the canonical STT + LLM + realtime-avatar composition used by the candidate, including stage latency, usage/cost counters and private-input/output digests/counts without serializing raw transcript, generated reply, signaling or credentials. A standalone TTS provider is not required when the selected realtime-avatar provider performs speech synthesis as part of its browser-connected spoken-avatar path; audible playback and rendered-video proof still come from the browser Owner Lab session evidence. Raw sanitized `rt0-owner-lab-session-evidence-0.6` JSON snapshots are discovered in the evidence directory, matched by exact byte digest to the ordered `snapshot_sha256` list in the bound aggregate, and recomputed into the canonical session binding. Each snapshot carries the server-derived owner/visitor participant role; inventory `0.7` reuses the canonical conversation-binding validator so the credentialed conversation receipt and raw role-bound snapshots must reproduce the owner/visitor Russian, completed-turn, playback/voice, rendered-video, and interruption claims in the exit manifest. Missing, tampered, duplicate, stale, cross-candidate/provider, detached supporting claims, or extra/unbound session snapshots keep the inventory incomplete. Snapshot filenames are not trusted as evidence identity. It does not evaluate conversation quality, human review, latency thresholds, privacy outcomes, or release readiness.

Use canonical filenames in the private evidence directory and run:

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-evidence-inventory -- \
  /secure/evidence/rt0-candidate \
  "$(git rev-parse HEAD)"
```

Exit `0` means only that every expected inventory slot is present, `exit-evidence.json` is structurally bound to the presented core/supporting artifacts for the exact candidate/provider state, the external runtime receipts agree with those bindings, and the raw session snapshots exactly reproduce the archived bound session aggregate. The canonical inventory requires the exact `release-spec.md` and `private-golden-evidence.json` bytes, `conversation-attempt.json`, `bound-session-aggregate.json`, all ten supporting-evidence files, and every raw session snapshot named by that aggregate's `snapshot_sha256` list; malformed, stale, cross-candidate, cross-provider, detached, missing, tampered, duplicate, or unbound evidence keeps the inventory incomplete. The JSON field is deliberately named `inventory_complete`; the tool never emits a `ready` claim. Exit `1` means files are missing or core bindings do not match. Exit `2` is command/input failure. A complete inventory must still pass `vpr-rt0-exit-evidence` and the underlying artifacts must genuinely represent the real evidence they claim.

## RT0 supporting-evidence scaffold

Before a real RT0 evidence session, create the canonical ten-file supporting directory with exact
candidate/provider-state bindings:

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-supporting-scaffold -- \
  /secure/evidence/rt0-candidate/supporting \
  /secure/evidence/rt0-candidate/provider-state.json \
  "$(git rev-parse HEAD)"
```

The scaffold is intentionally **non-promoting**. CI/E2E start as `failed`; every real-evidence JSON
starts with `origin="synthetic"`; conversations have zero completed turns; human-review dimensions
are `missing`; cost is unmeasured; and `known-limitations.md` starts with
`RT0-Review-Status: failed`. The command refuses to overwrite any existing canonical supporting
file.

Replace those placeholders only from reviewed observations for the exact candidate/provider state.
Until then, `vpr-rt0-supporting-preflight` must fail. The scaffold exists to eliminate filename,
binding and field-shape mistakes, not to manufacture evidence.

## RT0 runtime-backed supporting projection

After the exact owner/visitor browser sessions have been exported and bound, derive the three
supporting claims that are mechanically supported by runtime evidence instead of copying them by
hand:

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-runtime-supporting -- \
  /secure/evidence/rt0-candidate/supporting \
  /secure/evidence/rt0-candidate/provider-state.json \
  /secure/evidence/rt0-candidate/conversation-attempt.json \
  /secure/evidence/rt0-candidate/bound-session-aggregate.json \
  "$(git rev-parse HEAD)" \
  /secure/evidence/rt0-candidate/session-owner.json \
  /secure/evidence/rt0-candidate/session-visitor.json
```

The command recomputes the bound session aggregate from the exact raw snapshots and refuses stale,
cross-candidate, cross-provider or detached inputs. It writes only
`owner-conversation.json`, `visitor-conversation.json` and `quality.json`. If those files still
match the exact synthetic scaffold bytes for the same candidate/provider state, the command replaces
only those placeholders. Any modified, reviewed or real existing file is never overwritten. and uses the same canonical derivation logic as the exit verifier for Russian
locale, completed turns, canonical playback/voice, rendered video, owner interruption and all six
session-backed QualityContract latency distributions.

This command is intentionally non-promoting. It does not infer acceptance, privacy/permission,
cost, Golden or human-review results because those require separate real observations or review.

## RT0 supporting-evidence preflight

Before assembling `exit-evidence.json`, `vpr-rt0-supporting-preflight` can validate the ten
supporting artifacts as a non-promoting typed preflight:

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-supporting-preflight -- \
  /secure/evidence/rt0-candidate/supporting \
  /secure/evidence/rt0-candidate/provider-state.json \
  "$(git rev-parse HEAD)"
```

The preflight requires exact candidate binding for CI/E2E and exact candidate + provider-state
binding for owner/visitor conversation, acceptance, quality, cost, privacy/permissions and human
evaluation. The seven real-evidence slots must declare `origin=real`; owner/visitor roles are
checked; latency distributions must be structurally valid; human review must contain a non-empty
rubric, at least one reviewer and all five mandatory recorded dimensions. `known-limitations.md`
must be valid UTF-8, must contain at least one non-empty body line, and its first non-empty line must
be exactly `RT0-Review-Status: passed` or `RT0-Review-Status: failed`. The preflight reports that
status without promoting it.

A successful report emits exact SHA-256 digests for all ten supporting artifacts and
`preflight_complete=true`. It deliberately has no `ready` field and does not turn failed
acceptance/privacy/usability results into passes. The final exit gate remains responsible for the
substantive RT0 thresholds and for binding these artifact bytes to `exit-evidence.json`.

## RT0 exit-manifest assembler

After the core and supporting artifacts have been captured, use the non-promoting assembler instead
of manually copying statuses and SHA-256 values into `exit-evidence.json`:

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-exit-assemble -- \
  /secure/evidence/rt0-candidate/supporting \
  /secure/evidence/rt0-candidate/bound-golden-report.json \
  /secure/evidence/rt0-candidate/provider-state.json \
  /secure/evidence/rt0-candidate/provider-probe.json \
  /secure/evidence/rt0-candidate/conversation-attempt.json \
  /secure/evidence/rt0-candidate/bound-session-aggregate.json \
  docs/releases/RT0_RELEASE_SPEC.md \
  /secure/evidence/rt0-candidate/exit-evidence.json \
  "$(git rev-parse HEAD)"
```

The assembler first runs the canonical supporting-evidence preflight, validates the bound Golden
report against the exact candidate, ReleaseSpec bytes and provider state, validates the live-provider
probe through the canonical probe validator, and reuses the canonical conversation-attempt and bound
session-aggregate validators. It also reuses the canonical browser-quality binding, so the presented
quality claim must already match the bound aggregate for first audio, interruption stop, first video,
A/V sync and reconnect before a manifest is emitted. It then projects the exact supporting JSON bytes
into the canonical `Rt0ExitEvidence` claim types, computes every artifact digest itself, and derives
the known-limitations review status from the exact Markdown marker.

The assembler does **not** evaluate release readiness and has no `ready` output. Failed CI,
acceptance, privacy, human-usability or limitations-review results remain failed in the assembled
manifest. The output path must not already exist; the CLI writes a temporary sibling file and commits
it by same-directory rename only after complete successful assembly, so a failed assembly does not
publish a partial manifest.

The generated manifest must still pass `vpr-rt0-exit-evidence`; assembly is only an integrity and
operator-error reduction step, never RT0 exit evidence by itself.

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

The exit checker recomputes the sanitized provider-state digest from the separate provider-state file and requires its parsed contents to exactly match the provider state embedded in the Golden report. It also recomputes the SHA-256 of the separate credentialed live-provider probe, credentialed owner/visitor conversation-attempt receipt, and bound Owner Lab session aggregate, and requires all three artifacts to be bound to the same exact candidate and provider state. The release CLI additionally requires the canonical supporting-evidence directory containing `ci-evidence.json`, `e2e-evidence.json`, `owner-conversation.json`, `visitor-conversation.json`, `acceptance.json`, `quality.json`, `cost.json`, `privacy-permissions.json`, `human-evaluation.json`, and `known-limitations.md`. Its verified entry point hashes the exact bytes of all ten files and requires each digest to equal the corresponding digest declared inside `exit-evidence.json`. For RT0 cost evidence, the release contract additionally requires explicit, independent coverage for each cost signal across all three material provider roles (`stt`, `llm`, and `avatar`). Estimate coverage and provider-charge coverage are never merged: a duration plus STT/LLM estimate and an avatar-only provider charge are still two incomplete signals and cannot close RT0. Provider charge and estimate remain separate signals; neither is invented when a provider does not expose it.

For the nine JSON supporting artifacts it then parses those same bytes and requires exact equality with the canonical supporting projection. Every projection omits only `artifact_sha256`; CI/E2E additionally carry `candidate_sha`, while owner/visitor conversation, acceptance, quality, cost, privacy/permissions, and human-evaluation projections carry both `candidate_sha` and `provider_state_sha256`. Reusing passed automation from another commit or real evidence from another candidate/provider state therefore fails structurally even after an honest digest rebind. `known-limitations.md` remains a reviewed document bound by its exact byte digest rather than a JSON projection; additionally, its first non-empty `RT0-Review-Status: passed|failed` marker is parsed from those exact bytes and must equal the manifest `review_status`, so review status cannot be flipped independently and rescued by rehashing the document. One or more raw sanitized Owner Lab session snapshots are mandatory verifier inputs: the gate recomputes the bound aggregate from those exact bytes, candidate and provider state, then requires exact equality with the archived bound aggregate. Runtime-backed `canonical_playback_proven=true` and A/V-sync proof are accepted only through that recomputation. For session-derived quality, the recomputed aggregate must exactly equal the supporting QualityEvidence distributions for `text_first_meaningful_response`, `first_meaningful_audio`, `interruption_stop`, `first_useful_video`, `av_sync_absolute_offset`, and `recoverable_reconnect`; a detached or hand-edited distribution is rejected structurally before thresholds are evaluated. Text first-response timing comes from the backend-observed first non-empty streaming LLM chunk on a canonical text turn; the other five metrics remain browser/media-derived. A/V sync additionally requires every completed playback turn to have the required three request-scoped samples. A valid rehashed probe from another commit or provider configuration is rejected structurally. It also re-runs the bound Golden evaluator over the private Golden evidence bundle using the compiled mandatory `rt0_golden_minimum.json` suite, the exact ReleaseSpec bytes, provider state, and candidate SHA; the recomputed report must exactly match the archived sanitized Golden report. This prevents shortened/forged Golden reports and cross-provider/model/representation reuse.

Exit codes are stable: `0` means the supplied exact-candidate evidence satisfies the deterministic gate, `1` means evidence is structurally valid but one or more mandatory RT0 conditions fail, and `2` means evidence is malformed, stale, tampered, or cross-candidate. `docs/evaluation/rt0_exit_evidence.synthetic.example.json` is schema documentation only; every substantive origin in it is `synthetic`, so it is intentionally ineligible for RT0 exit.

Even a `0` from this experimental checker does not by itself promote `release.rt0_exit_gate`: the archived evidence artifacts must actually exist and correspond to the real provider/model/representation state named by the bound Golden report.

## Owner Lab session-evidence aggregation and binding

`vpr-rt0-session-aggregate` consumes one or more sanitized `rt0-owner-lab-session-evidence-0.5` JSON snapshots and deterministically derives the latency distributions that those snapshots can actually support: canonical text first meaningful response, STT, LLM, avatar-submit, server-total, browser-observed first audio, interruption stop, first rendered video, reconnect restoration, and request-scoped A/V sync absolute offset when WebRTC estimated playout timestamps are available. It also sums estimated/provider cost only when every completed provider stage contains that cost field; partial cost never becomes a fake complete total.

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

The bound receipt schema is `rt0-owner-lab-session-aggregate-binding-0.5`. It includes the exact candidate SHA, SHA-256 of the exact provider-state bytes, SHA-256 of every raw snapshot artifact in input order, and the deterministic aggregate. Snapshot schema `0.5` also carries payload-redacted canonical text attempts, their runtime-issued turn/output sequences, first-meaningful-response timing, LLM usage, and the participant role assigned by the Owner Lab server when that canonical owner/visitor session starts; filenames are never used to infer role. Provider-state structure is revalidated and duplicate/malformed snapshot artifacts fail closed.

Binding does not let observations self-promote. In schema `0.5`, a completed text attempt carries runtime-issued canonical turn/output sequences plus backend-derived first-meaningful-response timing, while a completed voice attempt carries the runtime-issued canonical turn and output-segment sequences; `audio_started` can mark that attempt playback-confirmed only after the backend reconciles the exact `OutputDeliveryHandle` to runtime `Played`. Request-scoped A/V sync samples use the explicit `web_rtc_estimated_playout_timestamp` reference and are accepted only after that same request has canonical playback confirmation. The aggregate sets `canonical_playback_proven=true` only when every completed attempt has playback proof, and `av_sync_proven=true` only when every completed attempt also has all three valid sequence-numbered A/V sync samples. The RT0 exit checker recomputes this aggregate from the raw snapshots and requires exact equality for text first meaningful response plus every browser-derived QualityEvidence distribution it can authoritatively support: first meaningful audio, interruption stop, first useful video, reconnect restoration, and A/V sync. Only after that exact-byte-derived binding succeeds are the ReleaseSpec thresholds evaluated. This closes the text/browser latency evidence-integrity chain; it still does not bind total session cost because the current aggregate does not expose a semantically complete source for that claim, and it does not by itself complete RT0 because real owner/visitor, privacy, cost, Golden and human-review evidence remain separately mandatory.
