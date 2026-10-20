# RT1 entry readiness and first vertical implementation sequence

This is a **bounded pre-entry feasibility deliverable**. It does not implement an
owner account, publish a public URL, or turn the RT1 capability on.

## Current evidence boundary

The owner has accepted the **RT0 functional demonstration** (Russian speech,
manual STOP, continuation, owner/visitor sessions) on candidate
`c393bae4cb369c82ef3f9160b9ba5fc6dc2d5774`.
See [OWNER_FUNCTIONAL_ACCEPTANCE.md](../release-evidence/rt0/OWNER_FUNCTIONAL_ACCEPTANCE.md).

The RT1 ReleaseSpec still says `BLOCKED_ON_RT0_EXIT`. Owner acceptance
must not be mistaken for a canonical release exit verdict. In particular,
interrupted bill-saving calls do not demonstrate full playout, A/V sync,
rights-cleared voice, final quality, security or cost qualification.

Run `python scripts/rt1_entry_readiness.py` to see the deliberately conservative
machine-readable entry state. A green script execution confirms that the
repository's **entry policy was recognized**, not that release qualification
passed. Only the exact-evidence exit verifier can confirm formal RT0 exit.

## First RT1 vertical: implementation order after authorized entry

| Slice | End-to-end observable outcome | Canonical source; no second brain |
| --- | --- | --- |
| 1. Workspace + draft | Authenticated owner creates one canonical Persona in their workspace and can reopen it | `vpr-domain` identity, `vpr-policy` authority, one durable repository/commit |
| 2. Capture/review | Owner provides sources, confirms/corrects claims; future answers use the latest approved version | Existing guided capture, claim provenance and `PersonaVersion` |
| 3. Prepared modalities | Text/voice/video readiness separated; retry voice/video failures without losing Persona | Existing provider-neutral ports; dedicated preparation job state |
| 4. Visitor preview | Owner launches visitor-scoped preview and verifies private facts stay private | Same runtime, visitor authorization; never owner privileges |
| 5. Publication + cost | Owner sees bounded cost estimate and publishes an independently shareable link | Version-bound publication pointer, current authorization check before provider egress |
| 6. External visitor | New person opens link and conducts real text, voice and video conversation | Canonical session/turn; visitor identity independent of owner |
| 7. Pause/revoke | Owner pauses/unpublishes, new public sessions denied; existing leases invalidated | Current authorization epoch and publication version, fail-closed |

**Acceptance proof required:** happy path; owner correction reflected in new
answers; one failed preparation recovered without affecting other modalities;
visitor denial after pause/revoke; owner/private-data isolation; actual
quality and cost evidence; rollback/recovery; reviewed consent and voice rights.

## Before enabling public access

Complete the RT0 exact-candidate release inventory/reviewer evidence; then
explicitly update the versioned RT1 ReleaseSpec entry status and replace
research-only fixtures with production Rust contracts, storage and end-to-end
tests. A developer-only preview is not an independently published product.

**Do not** implement an alternative Persona store, a second policy router,
mock public sessions passed off as real, or any implicit commercial approval
of the interim Irina voice.
