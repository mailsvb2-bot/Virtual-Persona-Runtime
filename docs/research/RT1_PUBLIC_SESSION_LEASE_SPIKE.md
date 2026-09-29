# RT1 public session lease / revocation — bounded pre-entry feasibility spike

**Status:** RESEARCH_REQUIRED  
**Release train:** RT1 pre-entry only  
**Production endpoint:** none  
**Durable RT1 session lease:** not implemented  
**Normative contract:** `docs/releases/RT1_RELEASE_SPEC.md`

## Question

Can an already-active public visitor session lose authority promptly and safely when publication, consent, permission, or PersonaVersion changes, without treating a provider session or client token as the source of truth?

## Result

Yes, if the future public-session lease is only a bounded authorization projection. It must bind the publication, canonical Persona identity/version, authorization epoch, publication-state version, and expiry. A provider session ID may help reconcile an external side effect, but it never grants authority by itself.

Issuance is allowed only from the exact current `PUBLISHED` state after server-side authorization. The lease is revalidated before each new turn or protected provider egress.

## Revocation semantics

`PAUSED`, `UNPUBLISHED`, effective consent/permission revoke, lease expiry, or a PersonaVersion change invalidates the old public-session authority.

The server-side denial does not wait for the browser to disconnect and does not depend on the provider successfully closing its own session. Provider disconnect is best effort; canonical authority remains local policy/state.

For a turn already in flight, revocation cancels or drains execution without further protected provider egress. A late provider success may be reconciled as non-current evidence but cannot restore authority.

## Concurrency rule

Authorization/publication epochs are monotonic. A stale resume racing with a newer revoke cannot re-authorize the older lease. Retrying lease issuance cannot create duplicate authority grants, and an old lease cannot be reissued as current without a fresh authorization check.

This closes the gap between “deny new bootstrap” and “revoke an already active public session”.

## Non-goals

This spike does not:

- add a public HTTP endpoint or link;
- implement durable session leases;
- promote publication to IMPLEMENTED or USER_REACHABLE;
- create provider-owned publication identity;
- weaken RT0 exit criteria;
- claim real revoke latency evidence.

## Executable proof

`tests/architecture/check_rt1_public_session_lease_spike.py` validates the machine contract at `contracts/rt1-public-session-lease-0.1.json`, composes it with the existing publication boundary, and keeps the spike locked to RESEARCH_REQUIRED while RT1 remains blocked on RT0 exit.
