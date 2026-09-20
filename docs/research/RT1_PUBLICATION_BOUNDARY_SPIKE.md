# RT1 publication boundary — bounded pre-entry feasibility spike

**Status:** RESEARCH_REQUIRED  
**Release train:** RT1 pre-entry only  
**Production endpoint:** none  
**Durable RT1 publication state:** not implemented  
**Normative contract:** `docs/releases/RT1_RELEASE_SPEC.md`

## Question

Can RT1 later expose a stable publication identity without turning publication into a second Persona brain or provider-owned identity?

This spike answers only the contract-level uncertainty identified by the RT1 ReleaseSpec. It does not implement publication, a public link, persistence, tenant administration, billing, or a user-reachable RT1 capability.

## Result

The boundary is feasible with a publication record that contains only:

- a stable `publication_id`;
- canonical `PersonaId`;
- canonical `PersonaVersion`;
- publication lifecycle state and authorization metadata when RT1 implementation is later eligible.

The publication record does **not** contain Persona claims, prompts, memory, provider/model identity, provider session identity, voice/avatar provider IDs, or copied representation truth.

A visitor bootstrap therefore resolves publication -> exact canonical Persona identity/version -> current authorization/policy/runtime. Publication is a pointer/control boundary, not a Persona source of truth.

## State contract

The spike validates:

`PRIVATE -> PREVIEWABLE -> PUBLISHED -> PAUSED -> PUBLISHED`

and terminal public withdrawal from either active public state:

`PUBLISHED|PAUSED -> UNPUBLISHED`

Unknown/ambiguous state fails closed.

## Scope separation

`OWNER_PREVIEW` and `PUBLIC_VISITOR` are distinct bootstrap scopes.

Owner preview may be available while a publication is PREVIEWABLE, PUBLISHED or PAUSED, subject to owner authority. Preview never grants public authority.

A new PUBLIC_VISITOR bootstrap is allowed only while state is exactly PUBLISHED and the publication's bound PersonaVersion still equals the current canonical version. PAUSED, UNPUBLISHED, stale PersonaVersion or ambiguous state deny before any future public provider egress.

This deliberately separates “the owner can preview the Persona” from “the public can start a new session”.

## Correction / stale projection rule

A publication is version-bound. If owner correction advances the canonical PersonaVersion, an older publication binding becomes stale and cannot bootstrap a new public session as current.

Future RT1 implementation may explicitly revalidate/rebind a publication after affected preparation/review work completes. It must not silently serve an older version as if it were current.

## Provider independence

Provider IDs are absent from publication identity. Replacing STT/LLM/speech/avatar representation providers must not change `publication_id`, `PersonaId`, or the meaning of `PersonaVersion`.

Provider/model/representation fingerprints may later appear in preparation/runtime evidence, but never as canonical publication identity.

## Non-goals

This spike intentionally does not:

- add Rust production publication types;
- add database tables or migrations;
- expose HTTP routes or a public link;
- create a second durable mutation path;
- make any RT1 capability user-reachable;
- claim IMPLEMENTED or PRODUCTION_READY;
- alter RT0 exit criteria or evidence.

## Executable proof

`tests/architecture/check_rt1_publication_spike.py` validates the machine-readable fixture at `contracts/rt1-publication-boundary-0.1.json` and cross-checks that the referenced canonical `PersonaId` and `PersonaVersion` remain defined in `vpr-domain`.

The proof is intentionally architecture-only. It must be deleted, replaced or promoted by a versioned RT1 production contract only after RT0 exit permits full RT1 implementation.
