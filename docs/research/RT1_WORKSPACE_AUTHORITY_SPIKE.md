# RT1 workspace / tenant authorization boundary — bounded pre-entry feasibility spike

**Status:** RESEARCH_REQUIRED  
**Release train:** RT1 pre-entry only  
**Account/workspace system:** not implemented  
**Public authentication:** not implemented  
**Normative contract:** `docs/releases/RT1_RELEASE_SPEC.md`

## Question

Can RT1 add owner accounts/workspaces and public visitors without creating a second authorization brain or allowing a publication token to become owner authority?

This spike answers that boundary only. It does not implement accounts, login, OAuth, sessions, WorkspaceId, tenant persistence or public links.

## First RT1 tenant model

For the first owner product, one workspace is the tenant boundary.

A future canonical `WorkspaceId` is distinct from:

- `PersonaId`;
- provider/model/session identity;
- publication/public-link tokens.

A Persona belongs to exactly one workspace in the first RT1 model. Workspace ownership is a resource-boundary check, not a replacement permission engine.

## Permission composition remains canonical

The existing `vpr-policy::EffectiveAuthority::compose` remains the only permission-composition rule. It already intersects authority layers and lets explicit deny win.

Future request authorization is conceptually:

1. identify the authenticated principal when the route requires one;
2. resolve the resource's canonical workspace;
3. fail closed if an owner operation targets another workspace;
4. evaluate normal `EffectiveAuthority` scope intersection;
5. validate authorization epoch/revocation/expiry;
6. apply consent and egress policy before any provider call.

Workspace match can narrow access. It can never widen authority.

Workspace IDs therefore MUST NOT be encoded into ad-hoc `AuthorityScope` strings as a substitute for resource ownership. A scope expresses the operation; canonical workspace identity expresses which tenant owns the resource.

## Owner versus public visitor

Owner routes require an authenticated owner principal, matching workspace and the required effective scope.

Public visitors are not workspace members. They enter through the publication boundary already proven by the publication spike.

A public link or publication token:

- is not an owner credential;
- does not grant workspace membership;
- does not grant owner scopes;
- cannot call owner mutation routes;
- cannot expose owner-private material;
- cannot override PAUSED/UNPUBLISHED or stale PersonaVersion denial.

Public visitor authority stays publication/session bounded.

## Cross-tenant fail-closed behavior

Before any external provider egress:

- owner request for a Persona in another workspace -> deny;
- public visitor attempt to call an owner route -> deny;
- matching workspace with missing/denied operation scope -> deny;
- stale/revoked/expired authority -> deny;
- paused/unpublished publication -> deny.

A matching workspace plus allowed scope is still not sufficient to call a provider: normal consent and egress gates continue to apply.

## Non-goals

This spike intentionally does not:

- add `WorkspaceId` or account types to production Rust;
- add authentication/session libraries;
- implement roles, invitations, organizations or teams;
- create tenant database tables;
- add public-link endpoints;
- alter current RT0 Owner Lab audience behavior;
- claim IMPLEMENTED, USER_REACHABLE or PRODUCTION_READY.

## Executable proof

`tests/architecture/check_rt1_workspace_authority_spike.py` validates the contract at `contracts/rt1-workspace-authority-0.1.json` and cross-checks the existing canonical policy behavior.

The guard also verifies that production domain code has not silently gained `WorkspaceId`/`TenantId` while this contract is still research-only. Full account/workspace implementation must therefore explicitly replace/promote this pre-entry boundary after RT0 exit.
