# Repository engineering handoff (read before changing code)

## Current milestone and workstream

- **R0 functional demonstration: ACCEPTED / CLOSED by the owner on 2026-10-20.**
  Accepted candidate: `c393bae4cb369c82ef3f9160b9ba5fc6dc2d5774`.
  Source of truth: `docs/release-evidence/rt0/OWNER_FUNCTIONAL_ACCEPTANCE.md`
  and `docs/releases/R0_FUNCTIONAL_MILESTONE_RELEASE.md`.
- **Active engineering workstream: RT1.** Complete the existing owner/visitor
  journey, safe publication dependencies, and reproducible real GPU-renderer
  evidence. Repair regressions in accepted R0 behavior, but do not reopen the
  R0 demonstration or ask for repeated human STOP/continue demos.
- Distinguish **functional R0 accepted** from **formal commercial release
  qualification**. `release.rt0_exit_gate` has no verified PASS in this repo;
  commercial/production rights, real strict A/V, cost and human evidence remain
  independently gated. Never fabricate evidence or mark `PRODUCTION_READY`.
- The normative `docs/CANON.md` and versioned ReleaseSpecs remain binding.
  Nothing in this file bypasses RT1's formal public-release gate.

## Coding rules

- Work from fresh `main` and the actual canonical owners; do not build a
  second Persona store, policy engine, public router, worker or execution path.
- Include actionable code and tests, verify full owner -> visitor -> response ->
  STOP/revoke behavior and checks. Preserve existing RT0/RT1 functionality.
- For pending qualification, name the **exact missing evidence** instead of
  vaguely claiming "RT0 isn't done".
- Machine-readable development status: `python scripts/rt1_entry_readiness.py`.
