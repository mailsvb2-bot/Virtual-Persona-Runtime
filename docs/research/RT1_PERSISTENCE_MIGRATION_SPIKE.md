# RT1 persistence / migration boundary — bounded pre-entry feasibility spike

**Status:** RESEARCH_REQUIRED  
**Release train:** RT1 pre-entry only  
**Production database:** none  
**Migration executed:** no  
**Normative contract:** `docs/releases/RT1_RELEASE_SPEC.md`

## Question

Can RT1 later introduce durable state without treating today's RT0 Owner Lab process state, provider sessions or release evidence as a hidden first production database?

This spike answers only that boundary question. It does not choose PostgreSQL/SQLite, create schema migrations, add repositories, persist user data or make any RT1 capability user-reachable.

## Current RT0 reality

The current repository has no production durable database dependency and no RT1 application schema. It does, however, deliberately persist one bounded RT0 artifact: the **reviewed owner Persona snapshot** used to restore owner-reviewed material after a local Owner Lab process restart. On Windows that snapshot is stored in the current user's Credential Manager through the versioned chunked secure-store boundary; the explicit test/development file path remains an opt-in alternative.

Everything else that defines a live runtime remains process-local: active session/avatar handles, turn state, provider session/stream identifiers, consent/authority state, preparation jobs, counters and media transport material. Restart recovery of the reviewed Persona snapshot therefore does **not** make RT0 runtime/session state a production database.

The reviewed snapshot is an experimental local bootstrap artifact, not the future RT1 durable schema. It must still enter RT1 only through an explicit owner-controlled migration/bootstrap that re-establishes authorization, current review and PersonaVersion. Therefore there is no legitimate automatic RT0 database migration to perform.

## Migration decision

RT1 starts with a clean durable schema only after RT0 exit allows full RT1 implementation.

RT0 material is treated as follows:

- owner-reviewed material may enter RT1 only through an explicit owner-controlled bootstrap/import path that re-establishes authorization, review and current PersonaVersion;
- RT0 session IDs, turn IDs, provider session/stream IDs, WebRTC SDP/ICE material, ephemeral consent/authority and RT0 preparation-job identity are discarded from product state;
- RT0 live-proof, browser and release-evidence artifacts remain immutable evidence/archive material and MUST NOT become application truth;
- ambiguous state fails closed instead of being guessed into a durable record.

This avoids converting experimental process memory or test evidence into hidden production state.

## Future durable ownership

A future RT1 durable model may store only the minimum concepts already named by the RT1 ReleaseSpec: workspace/tenant scope, Persona and PersonaVersion, reviewed claims and revision history, VoiceIdentity, AppearanceIdentity, provider-neutral representation bindings, preparation jobs/results, publication state, consent/rights and audit records.

Provider/session identifiers may be representation metadata where required, but never Persona identity.

## Atomic mutation boundary

The future implementation must have one canonical durable commit path.

For owner correction, the durable boundary must atomically preserve the old claim revision, create the new reviewed revision, advance PersonaVersion and write audit evidence. A partial commit that advances one but not the others is invalid.

Preparation/publication projections are version-bound. After correction, stale projections fail closed until explicitly recomputed/revalidated.

Retryable durable mutations require idempotency semantics so an uncertain retry cannot create duplicate Persona versions, duplicate claim revisions or duplicate publication transitions.

The RT0 local reviewed-Persona store now uses commit-or-rollback semantics for owner claim correction: an unsuccessful durable save restores the exact canonical profile revision history and modality-readiness state. This is a safety property of the experimental local store only; it is not a substitute for the future RT1 durable transaction/audit model.

## Rollback

Rollback or safe roll-forward must preserve Persona identity and claim-revision history. On ambiguous publication recovery, the safe state is private/not-public rather than guessed-public.

Provider replacement is not a Persona migration.

## Non-goals

This spike intentionally does not:

- add a database dependency;
- select a database engine;
- add tables, repositories or migrations;
- serialize the current in-memory PersonaProfile as a production storage format;
- create import/export product APIs;
- migrate RT0 evidence into user state;
- claim IMPLEMENTED, USER_REACHABLE or PRODUCTION_READY;
- change RT0 exit requirements.

## Executable proof

`tests/architecture/check_rt1_persistence_spike.py` validates the machine-readable fixture at `contracts/rt1-persistence-migration-0.1.json`.

It also scans every Cargo manifest and fails if a database crate is introduced while this pre-entry contract still declares `durable_database_implemented=false`. That keeps the spike truly non-production until RT0 exit and a versioned RT1 implementation decision explicitly replaces this research guard.
