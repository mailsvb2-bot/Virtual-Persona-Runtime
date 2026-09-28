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
- can Visitor Preview execute with visitor permissions while owner initiation remains separate from session authority;
- can preview and public bootstrap scopes remain distinct;
- can a future durable schema represent publication without embedding provider identity.

The spike must remain non-production, non-public and non-promoting. Its purpose is to remove uncertainty before RT1 entry, not to ship RT1 early.

### Claim lifecycle feasibility boundary

A second bounded pre-entry spike may freeze the owner-claim lifecycle because RT0 currently proves explicit approval/correction but RT1 additionally requires hide/delete semantics and stale-derived-state invalidation.

This spike may define only the contract for:

- `PENDING_REVIEW -> CONFIRMED | CORRECTED | HIDDEN | DELETED`;
- correction revision history and PersonaVersion advancement;
- hide as non-disclosure without pretending it is deletion;
- delete as removal of active personal content with at most a minimal non-content erasure/tombstone record;
- invalidation or erasure of affected derived/public state;
- fail-closed handling of stale PersonaVersion and non-owner mutation attempts.

It must remain contract-only and non-promoting while RT1 is blocked on RT0 exit. In particular it MUST NOT add production hide/delete routes, durable product-state mutations, public projections or a second claim authority before RT1 entry.

### Behavior training feasibility boundary

A bounded pre-entry spike may also freeze the behavior-training-by-example contract required by Canon §12 without making the capability production-reachable before RT1 entry.

The contract follows:

`Question -> Persona answer -> Owner feedback -> BehaviorChangeProposal -> Before/after preview on several examples -> Owner approval -> New PersonaVersion`

The active Persona remains unchanged until explicit owner approval. A proposal is candidate state only; rejection or abandonment leaves the active Persona unchanged.

Feedback that is really about facts, attribution or policy boundaries must not be smuggled into a style rule:

- wrong fact routes through canonical owner-claim correction authority;
- wrong boundary cannot widen policy, consent or delegation;
- “do not attribute this opinion” cannot create a verified owner belief;
- a preferred answer may be supplied by text or voice but does not silently become a verified fact/opinion.

Approval must bind the exact base PersonaVersion, fail closed when stale, create a new PersonaVersion instead of mutating the prior version in place, remain auditable, and be idempotent where retried.

This spike must remain contract-only and non-promoting while RT1 is blocked on RT0 exit. It MUST NOT add production BehaviorChangeProposal runtime/state, durable behavior mutations or owner-facing production controls before RT1 entry.

### Answer Evidence Card feasibility boundary

A bounded pre-entry spike may freeze the provenance UX contract required by Canon §33 without making the card production-reachable before RT1 entry.

The card is a disclosure-filtered projection of the real generation/evidence trace for the exact answer or Turn. It may expose sources used, owner-verified material, system inference, research date, freshness and uncertainty only when those facts are supported by the underlying trace.

The boundary must preserve these invariants:

- the card binds the exact PersonaVersion and actual answer/Turn evidence;
- owner-verified material remains distinct from system inference and simulated content;
- effective audience permissions apply to the card exactly as they apply to the underlying source material;
- the card cannot widen source disclosure, duplicate secrets or expose private source payloads by default;
- missing trace data is shown as unknown/unavailable rather than fabricated;
- revocation or deletion cannot be bypassed by reopening historical provenance UI;
- a voice answer to “Where do you know this from?” uses the same real trace and must not invent a post-hoc explanation unsupported by it.

This contract does not require exposing hidden chain-of-thought. It requires human-readable provenance derived from structured evidence.

This spike must remain contract-only and non-promoting while RT1 is blocked on RT0 exit. It MUST NOT add a production AnswerEvidenceCard runtime, durable provenance schema or owner/visitor production UI before RT1 entry.

### Cost and hard-budget feasibility boundary

A bounded pre-entry spike may freeze the RT1 cost-estimate and hard-session-budget semantics without making paid/public use production-reachable before RT1 entry.

The owner-facing cost model must preserve:

`ESTIMATE != USAGE != PROVIDER CHARGE != CUSTOMER CHARGE`

The first RT1 estimate is required before publication or first chargeable use and must bind the supported candidate configuration and an explicit estimate basis/version. Material provider/model/representation changes make that estimate stale. Unknown components remain unknown; missing provider charge must never be invented.

A hard session budget must not be silently exceeded. Concurrency must use reservation or an equivalent atomic strategy for expensive operations so parallel work cannot overspend the remaining budget. Retries must be idempotent with respect to reservations/charges, and UNKNOWN external outcomes require reconciliation before another reservation or charge is attempted.

Budget exhaustion uses stable reason `BUDGET_EXHAUSTED`. Any degradation or fallback must be pre-approved by policy and must not bypass egress, authorization or the hard budget. Realtime paid media requires explicit idle timeout, grace period and reconnect window before production so abandoned sessions cannot create uncontrolled GPU/video spend.

This spike must remain contract-only and non-promoting while RT1 is blocked on RT0 exit. It MUST NOT add a production pre-publication estimate API/UI, hard-session-budget runtime or customer charging path before RT1 entry.

### Preparation Plane job feasibility boundary

A bounded pre-entry spike may freeze the RT1 durable Preparation Plane job contract while reusing the already-proven RT0 modality-readiness model rather than creating a second readiness authority.

The canonical job lifecycle is:

`QUEUED -> PREPARING -> VALIDATING -> READY | FAILED | CANCELLED | EXPIRED`

Optional wait states may include `WAITING_USER_INPUT` and `WAITING_PROVIDER`.

A future durable job must bind a stable job identity, Persona/input revision, affected modality or asset kind, and any external provider operation identity needed for reconciliation. Provider identity must remain representation metadata, never canonical Persona identity.

Failure or cancellation of one preparation job must not destroy the Persona or unrelated ready modalities. Retry must target only the affected representation, be idempotent where repeated, and reconcile `UNKNOWN_OUTCOME` before a duplicate external operation can be launched. Local cancellation does not imply provider cancellation or refund.

Late completion from an old PersonaVersion/input revision, or from a cancelled/stale job, must not silently promote current readiness. Material provider/configuration changes require revalidation before the result can be current.

This spike must remain contract-only and non-promoting while RT1 is blocked on RT0 exit. It MUST NOT add durable production preparation-job storage, production retry/cancel routes or owner-facing job history before RT1 entry.

### VoiceIdentity / AppearanceIdentity feasibility boundary

A bounded pre-entry spike may freeze the RT1 canonical identity/representation contract for voice and appearance without creating production identity managers before RT1 entry.

The boundary preserves the Canon invariants:

`VoiceIdentity != TTS provider voice ID`

`AppearanceIdentity != avatar provider ID`

and provider/renderer identifiers remain representation bindings rather than canonical identity.

Voice and appearance operations may include `CREATE`, `SELECT`, `IMPORT`, `CONNECT` and `AUTO_ROUTE`, but real-person voice/face/body representations require the applicable rights/consent state, reference validation and quality evaluation before activation.

A logical VoiceIdentity or AppearanceIdentity may acquire a new provider representation without creating a new Persona identity. Any provider migration that changes acoustic realization must be visible to the owner and re-evaluated. Appearance references require validation and quality evaluation before activation. Each embodiment class carries its own independent maturity state. A candidate representation is not active merely because a provider returned success.

Provider/representation migration must preserve compatibility checking, rights/consent re-check, quality comparison for every migration, explicit activation and a rollback point. For a real-person identity, materially different automatic substitution must not silently activate.

This spike must remain contract-only and non-promoting while RT1 is blocked on RT0 exit. It MUST NOT add production VoiceIdentity/AppearanceIdentity persistence, management APIs or owner-facing identity-manager UI before RT1 entry.

### Persona lifecycle feasibility boundary

A bounded pre-entry spike may freeze the RT1 Persona lifecycle contract while reusing the existing RT0 capture, review and modality-readiness authorities rather than creating a second state machine.

The forward lifecycle remains:

`DRAFT -> CAPTURED -> REVIEWED -> READY_FOR_PREVIEW -> READY_FOR_PUBLICATION -> PUBLISHED`

with side transitions:

- `PUBLISHED -> PAUSED -> PUBLISHED`;
- `PUBLISHED|PAUSED -> UNPUBLISHED`.

Provider or preparation success must never advance Persona lifecycle by itself. Review is required before preview readiness, preview is required before publication readiness, and publication must bind the exact current PersonaVersion, current authorization/consent/rights and the required ready modalities for the selected supported configuration.

A correction after review creates a new PersonaVersion whose lifecycle re-enters at `REVIEWED`, including when the prior version was `READY_FOR_PREVIEW`, `READY_FOR_PUBLICATION`, `PUBLISHED` or `PAUSED`. The corrected version must pass preview and publication gates again before republishing. Only affected readiness moves back to preparation/review; unaffected readiness may remain valid only when its exact binding is still current. Stale preview/publication projections fail closed, and previously published PersonaVersions remain immutable historical evidence.

Pause and unpublish deny new public sessions but do not delete the private Persona. Resume requires a current valid publication binding.

This spike must remain contract-only and non-promoting while RT1 is blocked on RT0 exit. It MUST NOT add a production RT1 Persona lifecycle store, lifecycle management API or owner-facing lifecycle manager before RT1 entry.

### Consent / rights feasibility boundary

A bounded pre-entry spike may freeze the RT1 consent/rights semantics required for voice/appearance capture and use without creating production consent storage or UI before RT1 entry.

Rights remain independent dimensions rather than one overloaded status:

- `RIGHTS_BASIS`;
- `CONSENT_STATE`;
- `COMMERCIAL_PERMISSION`;
- `USAGE_RESTRICTIONS`;
- `TERRITORY`;
- `EXPIRY`;
- `REVOCATION_STATE`.

Real-person voice/face/body references require explicit consent scope. Importing a reference does not prove ownership or consent, and reference/quality validation remain distinct from rights validation. Commercial use requires an applicable commercial permission rather than being inferred from general consent.

Current consent/rights must be checked before activation or sensitive use. Expired, revoked, territory-incompatible, use-restricted or commercially insufficient state fails closed. Provider success, preparation readiness or cached readiness cannot override rights.

PersonaVersion remains immutable while authorization/consent is mutable. Revocation of an affected representation must invalidate current authorization and deny new sensitive operations before provider egress without requiring a new PersonaVersion. RT2 will define active-session revocation SLA and offline/air-gapped propagation semantics.

Raw biometric material must not be retained indefinitely by default; retention remains purpose-bound and subject to deletion/consent policy.

This spike must remain contract-only and non-promoting while RT1 is blocked on RT0 exit. It MUST NOT add production consent/rights persistence, consent-management APIs, owner-facing consent UI or RT2 revocation-runtime semantics before RT1 entry.

### Owner Control Center feasibility boundary

A bounded pre-entry spike may freeze the owner-facing control projection required by Canon §11 without creating a second claim, provenance or audience authority.

The Owner Control Center remains a projection titled equivalent to “What my Persona knows and says about me” over canonical claims/revisions, provenance and audience policy. It may classify owner-facing material as facts, opinions, preferences, stories, biography, skills, restrictions, relationships and system inferences.

Existing actions `confirm`, `correct`, `hide` and `delete` MUST route to the canonical owner-claim lifecycle. The Control Center does not acquire a separate mutation engine.

Additional owner controls are bounded as follows:

- `restrict audience` narrows visibility in this flow, routes to canonical audience policy and invalidates affected public projection;
- `mark outdated` preserves attributable history, removes the item from current factual context and invalidates affected derived/public state without pretending to delete it;
- `reject inference` applies only to system-inferred/derived material, must not manufacture an opposite owner-verified claim, and invalidates affected derived/public state;
- `inspect provenance` is a read-only, disclosure-filtered projection of canonical evidence and must not expose restricted source payloads or hidden chain-of-thought.

The owner correction authority hierarchy remains:

`Explicit Owner Correction > Verified Owner Statement > Verified Source > System Inference > Unverified Model Knowledge`.

All mutations require owner authority and fail closed on stale PersonaVersion. The Control Center cannot widen authority, bypass claim lifecycle or evidence disclosure, or require provider/model IDs in the normal owner flow.

This spike must remain contract-only and non-promoting while RT1 is blocked on RT0 exit. It MUST NOT add production Owner Control Center persistence, production mutation APIs or owner-facing production UI before RT1 entry.

### Capture / import feasibility boundary

A bounded pre-entry spike may freeze the first-owner capture/import semantics required by Canon §4.1 and RT1 while reusing the existing RT0 guided interview, canonical claim review, VoiceIdentity/AppearanceIdentity, Preparation Plane and consent/rights boundaries.

Supported owner inputs may include:

- structured owner interview;
- voice samples;
- video/appearance samples;
- owner-provided documents or sources;
- preferred-answer examples;
- explicit boundaries and prohibited attributions.

The pipeline produces **candidate structured records** for identity, communication style, biography, preferences, owner opinions, expertise, pronunciation, behavior and Constitution clauses. Capture does not turn inferred material into owner-verified truth. Automatically inferred owner facts/opinions remain unverified until explicit owner review or another approved verification rule applies.

Capture completion and owner-review completion remain distinct. Corrections route to the existing canonical claim/behavior authorities rather than creating a second capture-owned mutation path. A preferred answer does not silently become a verified fact or opinion.

Voice samples route to VoiceIdentity/preparation; appearance/video samples route to AppearanceIdentity/preparation. Real-person references remain gated by current consent/rights, reference validation and quality evaluation. Provider IDs are not owner-facing identity, and reference assets do not become Persona claim truth.

Capture quality remains separate from embodiment quality. Failed voice preparation and failed avatar preparation must each preserve reviewed owner knowledge and Persona identity; text capture may remain valid while a media representation is failed or preparing.

Imported sources retain provenance/reference identity. Import does not prove consent or ownership. Raw voice/video retention remains purpose-bound and is not indefinite by default; sensitive references require stricter ACL.

Retry/completion semantics must prevent duplicate candidate/review effects for the same request identity, and uncertain external provider outcomes require reconciliation before unsafe retry.

This spike must remain contract-only and non-promoting while RT1 is blocked on RT0 exit. It MUST NOT add production RT1 capture/import persistence, production upload/import APIs or owner-facing production capture/import UI before RT1 entry.

### API contract feasibility boundary

A bounded pre-entry spike may freeze cross-cutting RT1 API semantics before any production RT1 transport is implemented.

The bounded operation families cover owner workspace/session identity, Persona create/read, capture/import initiate/complete, owner claim review/mutations, preparation jobs, readiness, Visitor Preview, publication controls, public visitor bootstrap, Answer Evidence Card retrieval, cost estimate, and owner-visible audit/provenance.

Mutations require a stable request identity so an exact retry cannot silently create a second business effect. Writes against mutable Persona/publication/claim state require an expected version or equivalent concurrency token; stale writes fail closed rather than overwriting newer owner state.

When an API operation can cause an external side effect, provider idempotency is used where supported, external operation identity is preserved where available, and `UNKNOWN_OUTCOME` is reconciled before an unsafe retry. Local cancel does not imply provider cancellation or refund.

List operations use bounded page sizes and an opaque cursor with explicit stable ordering. A cursor must not contain owner secrets/provider credentials and can never bypass current authorization or disclosure filtering.

The API contract is versioned. Breaking changes require an explicit version or migration/deprecation path, and saved publication links must not silently change meaning. Provider/model/vector-store identifiers are not required in the normal owner payloads.

Expected domain denial/failure uses stable reason-code families. Authorization or provider failure must not be reported as business success. Before production, each expected reason used by the first journey must have a user-actionable UI mapping.

Input validation is server-side. Unexpected enums, oversized payloads and ambiguous/duplicated fields must follow an explicit fail-closed or canonicalization contract. Visitor authority cannot call owner mutation surfaces.

This spike must remain contract-only and non-promoting while RT1 is blocked on RT0 exit. It MUST NOT add production RT1 endpoints, public API exposure or a durable mutation transport before RT1 entry.

### Stable reason-code / actionable UI feasibility boundary

A bounded pre-entry spike may freeze the first-owner UI semantics for expected failures and denials while reusing the existing canonical `Rt0ReasonCode` enum rather than creating an RT1-specific error brain.

The canonical reason identity remains the stable code, not localized copy. RT1-specific additions require a versioned contract change.

Each reason used by the first RT1 journey must map to a user-comprehensible blocking/recovery state before production. The UI mapping does not create authority: server-side authorization, consent, egress and budget decisions remain authoritative even if a retry or action button is visible.

Authorization and consent denials must not auto-retry without the required authority/consent change. Egress denials must not silently switch to external egress. Provider unavailable/rate-limit/timeout may offer retry or only a pre-approved fallback; fallback must not be silent. Preparation failure retries only the affected representation while unrelated valid modalities remain usable. Budget exhaustion stops chargeable work rather than overrunning the hard budget. Internal errors show a safe generic failure without raw secret/stack disclosure.

An unknown reason must never render as success. Provider failure, cancellation or invalid state must not be presented as completed business success.

This spike must remain contract-only and non-promoting while RT1 is blocked on RT0 exit. It MUST NOT add a second reason enum, production reason-catalog API or production UI mapping before RT1 entry.

### Owner-visible audit feasibility boundary

A bounded pre-entry spike may freeze the owner-visible audit semantics required by the first-owner journey without creating a second event store or a second Persona/claim authority.

The audit view is a projection of authoritative committed mutations and execution evidence. It must not become a second Persona or claim source of truth.

At minimum, future production audit must be able to represent material owner-visible events for Persona/version changes, claim confirm/correct/hide/delete, audience restriction, rejected inference, consent/rights change, voice/appearance activation, publication create/pause/resume/unpublish, preparation retry/cancel requests, approved behavior changes, and relevant security/authority denials.

Audit records bind stable event identity, actor/scope, Persona identity/version where applicable, target/request/operation identity where applicable, result, stable reason code where applicable, and revision/evidence references. They must not copy secrets or unrestricted private payloads by default.

Deletion audit must not force retention of deleted personal content: a minimal non-content tombstone/evidence record is sufficient where deletion evidence is required.

A retried mutation that resolves to one committed business effect must not create duplicate committed audit meaning. Durable state and any required audit event must not split-brain. Unknown external outcomes may be recorded as unknown/reconciling but not as success.

Owner audit reads remain subject to current authority/disclosure. Visitor authority cannot read owner audit. Answer Evidence Card remains the turn-specific provenance projection and is not replaced by this audit trail.

This spike must remain contract-only and non-promoting while RT1 is blocked on RT0 exit. It MUST NOT add a durable production audit log, owner-facing production audit UI or audit-export API before RT1 entry.

## 20. Exit gate

RT1 is complete only when:

- an owner unfamiliar with implementation details completes the full journey without developer assistance;
- at least one supported production candidate configuration completes a real independent visitor video conversation;
- correction, preparation failure/recovery and pause/unpublish paths pass;
- inherited RT0 quality/privacy/attribution guarantees remain satisfied;
- the product does not satisfy its video promise by leaving video permanently unsupported or `PREPARING`;
- the exact candidate has a complete reviewed RT1 ReleaseEvidence bundle.

Until then RT1 MUST NOT claim `PRODUCTION_READY`.
