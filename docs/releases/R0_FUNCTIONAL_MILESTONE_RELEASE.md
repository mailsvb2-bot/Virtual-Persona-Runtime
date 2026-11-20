# R0 functional milestone release — accepted and closed

**Milestone:** `R0-FUNCTIONAL-ACCEPTED-2026-10-20`  
**Decision:** `ACCEPTED_CLOSED` (owner sign-off 20 October 2026)  
**Accepted candidate:** `c393bae4cb369c82ef3f9160b9ba5fc6dc2d5774`  
**Next development workstream:** **RT1** (owner/visitor, safe publication, verified rendering).

## What is released

The owner-accepted **functional demonstration**: one-command Windows Owner Lab
start, real connected audio/video avatar, Russian voice conversation, STOP,
continuation, intentional session end, and owner plus distinct visitor roles.
The exact acceptance record is
[`OWNER_FUNCTIONAL_ACCEPTANCE.md`](../release-evidence/rt0/OWNER_FUNCTIONAL_ACCEPTANCE.md).

**Do not restart or repeatedly request this already accepted R0 demonstration
as a prerequisite for ordinary RT1 programming work.** New changes must
preserve these working mechanisms. Bugs affecting them are regressions, not a
reason to re-open the completed functional milestone.

## Separate release-qualification boundary

This is **not** a production/commercial RT0 release, a GitHub packaged binary,
or proof that the canonical `vpr-rt0-exit-evidence` verifier passed. Exact-candidate
full playback, strict A/V measurements, human quality, voice rights, cost,
privacy and Golden evidence remain unverified as a **distinct qualification
workstream**. Never change `release.rt0_exit_gate` to PASS, mark
`PRODUCTION_READY`, or enable public RT1 publication from this decision alone.

The RT1 ReleaseSpec's existing formal gate remains authoritative until a
separately reviewed real evidence bundle passes the canonical verifier.

## Instructions for subsequent coding sessions

1. Treat **R0 functional milestone = CLOSED** and use the accepted test path
   as regression baseline, not a newly pending delivery task.
2. Plan and implement bounded RT1 work in the existing canonical ownership
   chain; do not build a second Persona/authority, public endpoint or sender.
3. If work is blocked by an exact production requirement, identify **that
   requirement and missing evidence**, not the generic phrase "RT0 isn't done".
4. Distinguish functional milestone `ACCEPTED_CLOSED` from formal release
   qualification `NOT_VERIFIED`. Never silently promote the latter.
5. Keep RT0 regression tests, original acceptance record and release-spec
   intact. Changes to production gating require a separate documented review.

**Machine-readable handoff:** `python scripts/rt1_entry_readiness.py`.
