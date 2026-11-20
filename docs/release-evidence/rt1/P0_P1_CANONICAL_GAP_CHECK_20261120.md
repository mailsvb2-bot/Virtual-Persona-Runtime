# RT1 P0/P1 canonical implementation handoff — 2026-11-20

**Base commit:** `6fe956a713499571b6189fa52c589eb5b653fd1c`  
**R0 functional milestone:** owner-accepted and CLOSED; do not reopen.

## Remote branch integration findings

Before changing production paths we compared the current `main` with all 88
historical remote branch heads in an isolated clone, including the
`feat/rt1-*`, `fix/rt1-*`, `integrate/rt1-*` and RT1 test branches.

- `feat/rt1-publication-authorization-spike-20261021` (6 ahead / 181 behind)
  contains the original pure authorization guard now present in
  `crates/vpr-policy/src/publication_access.rs`.
- `feat/rt1-canonical-publication-lifecycle-20261021` (34 ahead / 180 behind)
  contains the original publication/Persona lifecycle already integrated
  in the current `rt1_workspace*.rs` and `publication_lifecycle.rs`.
- `feat/rt1-unified-owner-preview-workspace-20261021` (18 ahead / 183 behind)
  is the older single-workspace UI; it does not provide independent owner
  accounts and would overwrite newer tested frontend code.
- `feat/rt1-owner-http-boundary-20261023` (13 ahead / 170 behind) is an older
  version of the currently routed RT1 HTTP boundary.
- `feat/rt1-offline-persona-migration-preflight-20261020` (21 ahead /
  185 behind) is a historical, pre-entry migration branch, not tenant
  identity, HTTPS or an executed GPU acceptance.
- No branch introduces a **new path** for a complete owner-account database,
  approved public site deployment, licensed NVIDIA acceptance evidence, or
  canonical rights/budget runtime that is absent from `main`.

**Decision:** selectively use the current canonical implementations; do not
merge old branch heads or recreate a second Persona store, policy or sender.

## Exact software and operational status

| Requirement | Code present in main | Current missing release proof |
| --- | --- | --- |
| P0: independent owner accounts | Single reviewed Persona V2 store, single canonical owner workspace and owner secret | Tenant-aware canonical persistence, owner signup/login, isolation across accounts and active sessions |
| P0: HTTPS visitor edge | `deploy/rt1-public-visitor.nginx.conf.template`, `/api/rt1/visitor/bootstrap`, invitation fragment | Approved DNS + TLS + deployed Nginx, external no-leak / deny-route checks; use `scripts/rt1_public_edge_probe.py` |
| P0: MuseTalk CUDA and A/V | Canonical renderer and `scripts/rt1_musetalk_hardware_acceptance.sh` | NVIDIA driver/GPU, approved image/weights/consent, real frame and voice timings, browser lip-sync/STOP receipts |
| P1: biometric rights and consent | Contract `rt1-consent-rights-0.1.json` | Runtime implementation flag is **false**, no durable per-use commercial authorization |
| P1: budget/actual cost | Contract `rt1-cost-budget-0.1.json` | Real estimate and hard concurrent session budget flags are **false** |
| P1: release evidence | `docs/releases/RT1_RELEASE_SPEC.md`, software tests | Candidate-pinned reviewed real deployment, GPU, cost, rights, rollback and two-device acceptance |

None of the inspected authorized servers exposed a working NVIDIA GPU through
`nvidia-smi` on 2026-11-20. The repository does not contain an approved
public RT1 visitor DNS name or installed TLS certificate. **Do not redirect
traffic from an unrelated product or deploy on its production host.**

## Safe change in this PR

The current owner `/api/rt1/owner/publish` call retains the same verified
canonical controller, workspace, media readiness and voice provider path.
With **no public origin configured**, local RT1 publishing and its existing
browser tests behave unchanged. If an operator sets a public HTTPS visitor
origin before durable rights and concurrency-safe budgets are implemented,
the same route now fails closed with
`RT1_PUBLIC_RELEASE_QUALIFICATION_REQUIRED`, **before the provider
readiness or any new visitor bearer allocation**. The same canonical
visitor router denies stale prior invitations and all new visitor
START/ANSWER/ICE/TEXT/VOICE/MEMORY requests when the public-origin
qualification is missing; STOP/CLOSE and existing privacy operations
(consent, erase, credential revoke and rotation) deliberately remain
reachable to drain active workers and preserve data-subject controls. No configuration boolean pretends to certify a
real commercial release.

Use `scripts/rt1_release_candidate_inventory.py` for an exact-head,
ReleaseSpec-digested and explicitly **UNVERIFIED** maturity record; use
`scripts/rt1_public_edge_probe.py --origin https://approved.example.org`
only with a real approved hostname. A PASS from that HTTP-only probe is
**not** rights, GPU, cost, media STOP or production-release proof.

## Stop condition

Do **not** publish commercial RT1 to the Internet, invent GPU evidence,
claim that multi-owner support works, or mark RT1 production ready until
the outstanding real canonical implementations and external evidence exist.
Preserve RT0 and already verified local RT1 functionality.
