# ADR-0001 — Rust-first modular monolith for RT0

**Status:** Accepted for RT0  
**Date:** 2026-09-12

## Context

The Canon requires one canonical authority/state model while allowing parallel media/AI execution. It explicitly rejects premature infrastructure splitting and assigns canonical Persona, policy, session and persistence decisions to Rust.

RT0 must prove the user-visible digital-twin experience before platform expansion.

## Decision

Start as one Rust workspace with logical crate boundaries:

- `vpr-domain` — provider-free canonical types and state invariants;
- `vpr-policy` — authority composition and global egress decisions;
- `vpr-integration` — provider-neutral capability ports and evidence DTOs;
- `vpr-runtime` — orchestration authority and turn cancellation.

Python/GPU and TypeScript processes are added only when a real RT0 vertical slice needs them. They never become canonical Persona/policy/memory authority.

PostgreSQL/persistence is intentionally deferred until the first durable RT0 state slice has an explicit migration contract. This avoids inventing a schema before the required state transitions are executable.

## Consequences

- Provider SDK types cannot enter `vpr-domain`.
- External AI/media adapters depend inward on VPR contracts.
- `LOCAL_ONLY`/`DENY` are runtime barriers, not routing preferences.
- A media/provider crash can later be process-isolated without moving canonical authority.
- Service extraction requires measured scaling, security, GPU or failure-isolation need.

## Fitness checks

CI compiles/tests the whole workspace and rejects formatting/clippy failures. Architecture tests will be expanded when provider adapters land to assert dependency direction mechanically.
