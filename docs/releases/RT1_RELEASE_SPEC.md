# RT1 ReleaseSpec — First Owner Product

**Spec version:** `RT1-0.1.0-draft`  
**Status:** `BLOCKED_ON_RT0_EXIT`  
**Allowed pre-entry work:** `BOUNDED_FEASIBILITY_SPIKE_ONLY`  
**Normative parent:** `docs/CANON.md` v3.4  
**Maturity ceiling before RT0 exit:** `RESEARCH_REQUIRED`

## 1. Goal

Turn the RT0 feasibility proof into the first independently usable owner product without creating a second canonical Persona, permission, policy or durable-state authority.

The RT1 user outcome is:

`Create -> Capture -> Prepare -> Review -> Correct -> Preview -> Publish -> Visitor talks -> Pause / Unpublish`

An owner unfamiliar with provider/model implementation details must be able to complete this journey without developer assistance once RT1 is eligible for release.

## 2. Release-train ordering and entry condition

RT0 remains the active release train until its exact-candidate exit gate is satisfied.

Full RT1 implementation may begin only after all of the following are true:

- `release.rt0_exit_gate` has passed on one exact candidate under the RT0 ReleaseSpec;
- the RT0 ReleaseEvidence bundle is complete and reviewed;
- mandatory RT0 privacy, attribution, latency, cost, Golden and human-review criteria have passed;
- no accepted RT0 safety/reliability gate is being bypassed.

Before that point, RT1 work is limited to a bounded feasibility spike needed to remove major technical uncertainty. Such work MUST NOT:

- claim `IMPLEMENTED`, `USER_REACHABLE` or `PRODUCTION_READY`;
- create a second Persona brain, policy authority, publication authority or durable commit path;
- expose a real public production publication endpoint;
- silently migrate RT0 experimental state into a durable RT1 schema;
- weaken or reinterpret the unfinished RT0 exit criteria.

## 3. Exact user journey

1. Owner creates or opens an owner workspace.
2. Owner creates one supported Persona in the first production-supported mode.
3. Owner completes guided capture and/or imports the minimum supported references.
4. Preparation runs independently for text, voice and appearance/video.
5. Owner sees per-modality readiness and actionable failure/retry state.
6. Owner reviews what the Persona knows and says about the owner.
7. Owner confirms, corrects, hides or deletes owner claims.
8. Owner can train behavior by reviewed examples without bypassing provenance or attribution rules.
9. Owner previews the visitor experience under visitor permissions.
10. Owner sees a cost estimate before enabling public use.
11. Owner publishes the Persona through the first supported publication mechanism.
12. An independent visitor opens the publication mechanism and completes a real text/voice/video conversation.
13. Owner can pause or unpublish public access without deleting the private Persona.
14. After correction or pause, stale derived/public state must not continue serving as if still current.

## 4. Scope

RT1 implements only the Canon RT1 scope:

- full Persona creation for the first supported mode;
- owner account/workspace basics with tenant-scoped authorization;
- Persona versioning;
- VoiceIdentity and AppearanceIdentity;
- Preparation Plane job lifecycle;
- owner capture/import of voice and appearance references;
- Owner Control Center;
- “What my Persona knows and says about me” ledger;
- confirm/correct/hide/delete owner claims;
- behavior training by example;
- public/private audience split;
- visitor preview;
- public link or equivalent first publication mechanism;
- Persona pause/kill switch;
- basic Answer Evidence Card;
- first cost estimate before publication/use;
- readiness profile for text, voice and video.

Anything beyond this list remains outside RT1 unless separately promoted by a versioned ReleaseSpec change.

## 5. Canonical ownership and architecture boundaries

RT1 MUST reuse the existing canonical domain, policy, runtime and provider-neutral boundaries established in RT0.

The following remain invariants:

- one canonical Persona identity and version history;
- one canonical policy/authorization model;
- one canonical durable commit path;
- provider IDs remain representation bindings rather than Persona identity;
- public publication state cannot become a second source of Persona truth;
- visitor context must be derived from canonical state under visitor authorization;
- owner correction/revocation wins over stale caches, preparation outputs and active/public projections.

RT1 MUST NOT introduce a separate “owner product brain” beside the canonical runtime.

## 6. State machines

### Persona lifecycle

`DRAFT -> CAPTURED -> REVIEWED -> READY_FOR_PREVIEW -> READY_FOR_PUBLICATION -> PUBLISHED`

Allowed side transitions:

- `PUBLISHED -> PAUSED -> PUBLISHED`
- `PUBLISHED|PAUSED -> UNPUBLISHED`
- correction from `REVIEWED` or later creates a new PersonaVersion and may move affected readiness back to preparation/review states.

Deletion semantics are explicit and must distinguish private Persona deletion from public unpublication.

### Representation preparation

`QUEUED -> PREPARING -> VALIDATING -> READY | FAILED | CANCELLED`

Text, voice and appearance/video readiness are independent. Failure of one representation MUST NOT destroy Persona identity or unrelated ready representations.

### Publication

`PRIVATE -> PREVIEWABLE -> PUBLISHED -> PAUSED -> UNPUBLISHED`

A paused/unpublished Persona MUST deny new public sessions. Existing sessions follow explicit lease/revocation semantics; no implicit continued public authority is allowed.

### Claim review

Current owner claim revisions have explicit reviewed visibility state:

`PENDING_REVIEW -> CONFIRMED | CORRECTED | HIDDEN | DELETED`

Corrections create attributable revision history rather than rewriting prior evidence.

## 7. API contract

RT1 will define a versioned API surface for:

- owner workspace/session identity;
- create/read current Persona;
- guided capture/import initiation and completion;
- list/review/correct/hide/delete owner claims;
- create/list/retry/cancel preparation jobs;
- query text/voice/video readiness;
- visitor preview session;
- publication create/read/pause/resume/unpublish;
- public visitor session bootstrap;
- Answer Evidence Card retrieval for supported answers;
- pre-publication/use cost estimate;
- owner-visible audit/provenance minimum needed by this journey.

Provider names, provider model IDs, vector-store identifiers and low-level protocols MUST NOT be required from the normal owner flow.

Exact transport payloads, idempotency keys, pagination and concurrency controls must be frozen before the first RT1 production implementation slice that depends on them.

## 8. Persistence and migration

RT1 introduces the minimum durable schema required by the Canon for:

- owner workspace / tenant scope;
- Persona;
- PersonaVersion;
- reviewed owner claims and revisions;
- VoiceIdentity and provider-neutral voice representations;
- AppearanceIdentity and provider-neutral appearance representations;
- preparation jobs and results;
- publication state;
- consent/rights needed for the supported flow;
- audit records.

The migration plan must explicitly decide which RT0 experimental state is discarded versus forward-migrated. No RT0 test artifact or provider session identifier may silently become canonical durable identity.

Every durable mutation affecting Persona, claim, permission, consent or publication state must be auditable and idempotent where retried.

## 9. Provider and connector contract

RT1 may reuse or replace supported STT, LLM, speech and avatar providers behind provider-neutral ports.

Provider replacement MUST NOT change:

- `PersonaId`;
- owner-reviewed claim identity/history;
- VoiceIdentity or AppearanceIdentity canonical identity;
- publication identity;
- authorization semantics.

Preparation results bind provider/model/representation versions and configuration fingerprints sufficient to detect stale or incompatible state without persisting secrets.

## 10. Privacy, consent and authorization

Effective authority remains intersection-based. Explicit deny, expiry, pause, unpublish and revocation win.

At minimum:

- owner routes are tenant-scoped;
- visitor preview and public visitor scopes never receive owner-private material unless explicitly audience-authorized;
- publication cannot widen claim visibility beyond reviewed audience policy;
- voice/appearance use requires applicable rights/consent state;
- pausing/unpublishing blocks new public sessions;
- owner correction/deletion invalidates affected derived/public state;
- secrets, raw biometric material and private source documents are not exposed in browser/public evidence by default.

No `LOCAL_ONLY` or `DENY` outcome may silently fall back to external egress.

## 11. UI states

The Owner Control Center must expose user-comprehensible states for:

- Persona creation/capture progress;
- claim review and current revision;
- text/voice/video preparation;
- preparation failure with affected-modality retry;
- preview readiness;
- estimated cost before public use;
- publication status: private, published, paused, unpublished;
- public access control / kill switch;
- provenance/evidence for supported answers;
- actionable authorization/consent failures.

The normal journey must not require the owner to configure providers, vector stores, protocols or model IDs.

## 12. Failure and recovery

Mandatory behavior:

- voice preparation failure does not destroy text/video readiness or Persona identity;
- video preparation failure does not destroy text/voice readiness or Persona identity;
- safe retry targets only the failed representation/job;
- provider timeout/unavailability emits a stable typed reason;
- correction invalidates stale derived/public state before it can be treated as current;
- pause/unpublish denies new public sessions even if a cache or publication projection is stale;
- uncertain external side effects are reconciled before unsafe blind retry;
- owner can recover the product journey without developer-only state surgery.

## 13. Stable reason-code families

RT1 reuses applicable RT0 stable reason codes and adds no new code casually.

Minimum families used by the RT1 journey include:

- `AUTH_REVOKED`
- `AUTH_EXPIRED`
- `AUTH_SCOPE_DENIED`
- `EGRESS_DENIED`
- `EGRESS_LOCAL_ONLY`
- `CONSENT_REQUIRED`
- `PROVIDER_UNAVAILABLE`
- `PROVIDER_RATE_LIMITED`
- `PROVIDER_TIMEOUT`
- `PREPARATION_FAILED`
- `INVALID_STATE_TRANSITION`
- `OWNER_ATTRIBUTION_UNVERIFIED`
- `BUDGET_EXHAUSTED`
- `INTERNAL_ERROR`

Any RT1-specific reason code must be added through a versioned contract change and mapped to user-actionable UI behavior.

## 14. QualityContract

RT1 preserves all accepted RT0 quality/privacy/attribution behavior. It may tighten thresholds but MUST NOT silently weaken them.

In addition, RT1 acceptance measures at least:

- end-to-end time from owner journey start to publishable Persona;
- publish completion success rate in the acceptance suite;
- visitor bootstrap and first meaningful response latency;
- text/voice/video readiness time and retry recovery;
- pause/unpublish propagation to public access;
- cost estimate availability before publication/use;
- accepted private-context leakage: 0;
- accepted false owner-opinion attribution: 0.

Exact numeric RT1 thresholds beyond inherited RT0 thresholds must be calibrated by measured RT1 feasibility evidence before promotion from this draft.

## 15. Cost and budget

Before publication or first paid/chargeable use, the owner must see a first cost estimate based on the selected supported production candidate configuration.

Evidence distinguishes:

- estimate;
- measured provider usage;
- confirmed provider charge when exposed by the provider.

Missing provider charge must not be invented. Budget exhaustion fails closed with a stable reason rather than silently switching to an unapproved provider/path.

## 16. Acceptance / E2E matrix

### Happy path

- owner creates a Persona;
- owner completes capture/import and review;
- all required modalities become ready;
- owner previews visitor behavior;
- owner sees cost estimate;
- owner publishes;
- independent visitor completes a real text/voice/video conversation.

### User correction path

- owner corrects a wrong claim or undesirable reviewed behavior;
- a new PersonaVersion/revision is attributable;
- affected derived/public state is invalidated/recomputed;
- future preview/public answer reflects the correction;
- prior evidence remains bound to the earlier revision.

### Failure + recovery path

- one voice or video preparation provider/job fails;
- the affected modality reports a typed failure;
- Persona and unrelated modalities remain intact;
- owner retries only the affected preparation;
- the journey can continue after recovery.

### Revoke / deny path

- owner pauses/unpublishes public access or applicable consent/permission is revoked;
- new public session creation is denied;
- private owner Persona remains available unless separately deleted;
- no protected provider call occurs after effective revocation for a new denied operation.

## 17. Release evidence

RT1 ReleaseEvidence must bind at least:

- exact Git candidate;
- this ReleaseSpec version/digest;
- schema/migration version;
- production candidate provider/model/representation state;
- unit/contract/integration/browser E2E;
- full owner journey result;
- independent visitor real conversation result;
- correction path;
- preparation failure/recovery path;
- pause/unpublish deny path;
- quality/latency measurements;
- cost evidence;
- privacy/permission results;
- known limitations;
- rollback/recovery proof.

Evidence from another candidate or materially different provider/configuration state is stale.

## 18. Rollback and migration

Every RT1 production migration must provide:

- forward migration;
- rollback or safe roll-forward strategy;
- explicit treatment of RT0 experimental data;
- no identity duplication during rollback;
- publication state recovery that defaults safe/private on ambiguity;
- no silent loss of reviewed claim revision history.

Provider replacement is never a Persona migration.

## 19. Pre-entry feasibility spike

While `Status = BLOCKED_ON_RT0_EXIT`, only bounded feasibility work is allowed.

The preferred first spike is contract-level validation of the publication boundary:

- can a stable publication identity point to canonical PersonaVersion without copying Persona truth;
- can pause/unpublish deny new visitor bootstrap fail-closed;
- can preview and public scopes remain distinct;
- can a future durable schema represent publication without embedding provider identity.

The spike must remain non-production, non-public and non-promoting. Its purpose is to remove uncertainty before RT1 entry, not to ship RT1 early.

## 20. Exit gate

RT1 is complete only when:

- an owner unfamiliar with implementation details completes the full journey without developer assistance;
- at least one supported production candidate configuration completes a real independent visitor video conversation;
- correction, preparation failure/recovery and pause/unpublish paths pass;
- inherited RT0 quality/privacy/attribution guarantees remain satisfied;
- the product does not satisfy its video promise by leaving video permanently unsupported or `PREPARING`;
- the exact candidate has a complete reviewed RT1 ReleaseEvidence bundle.

Until then RT1 MUST NOT claim `PRODUCTION_READY`.
