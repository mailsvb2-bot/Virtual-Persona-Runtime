# RT1 remote-branch reconciliation — 2026-11-20

**Base `main`:** `d21766aaeb5680038c6787d144f0bb29f8777f53`  
**Scope:** all 87 remote branches returned by `origin` (excluding `main`) at the snapshot.  
**Method:** the **existing** `scripts/audit_branch_diffs.py`, using a separate
read-only clone and complete `main..branch` file diffs, merge-base-introduced
patches, `git log --cherry-pick` and introduced-path inventory. No production
server, secrets or deployed state were modified. Branch/ref history differences
are not proof that functionality is missing from `main`.

## Results

- **87/87** remote branch trees differ from `main`, many by extensive older code;
  none qualifies as a safe content-identical fast-forward.
- Only **four** remote branches contain merge-base-introduced *paths* absent
  from `main`; the three RT1-relevant names are
  `fix/rt1-319-integrated-egress-20261026`,
  `fix/rt1-319-serialized-start-egress-20261025`, and
  `integrate/rt1-local-http-journey-20261104`.
  A fourth non-RT1 branch has unrelated legacy code. Path absence alone does
  not establish a missing behavior.
- `fix/rt1-319-integrated-egress-20261026` is **578 commits ahead and
  140 behind**, reflecting a wide pre-integration fork. Importing it would
  replace newer canonical HTTP, provider and visitor-stop boundaries.

## Disposition of missing RT1 files

| Old branch artifact | Decision |
| --- | --- |
| `tests/architecture/test_rt1_319_startup_dispatch_fence.py` | **PORT, ADAPTED**: original test assumes a single old startup implementation. New test covers current split `http_rt1_start.rs`, `rt1_visitor_conversation.rs` and `rt1_visitor_conversation_helpers.rs` and protects admission/STOP/SDP ordering. |
| `crates/vpr-owner-lab/src/rt1_visitor_voice_stream_staging.rs` | **DO NOT RESTORE**: obsolete pre-delivery staging path; main already uses canonical visitor voice worker / egress and interrupt fences. Reintroducing it would create an unused competing path. |
| `crates/vpr-provider-local-open-source/worker_lifecycle.py` and `test_worker_lifecycle.py` | **DO NOT RESTORE**: historic in-memory lifecycle in a different package; current durable operation worker and tests live under `scripts/rt1_media_worker_*.py`. A second lifecycle owner would weaken remote STOP reconciliation. |
| `.github/workflows/rt1-real-rtp-revoke.yml` | **DO NOT RESTORE**: newer `rt1-local-sender.yml` and `rt1-owner-visitor-canary.yml` run RTP and owner/visitor acceptance. |
| `crates/vpr-owner-lab/ui/e2e/rt1-livekit.spec.ts` | **DO NOT RESTORE**: current `rt1-livekit-provider-lifecycle.spec.ts` plus owner/visitor scenarios cover the supported route with newer revocation behavior. |
| `crates/vpr-owner-lab/src/bin/support/provider_credentials_echo_tests.rs` | **DO NOT RESTORE**: older RT0/provider-fixture support, not RT1 public feature; current credentials and RT1 worker boundaries supersede it. |
| `docs/release-evidence/rt1/RT0_RT1_VISITOR_SOURCE_TRACE.md` | **ARCHIVE IN BRANCH**: dated November 2, describes staging-only text/voice paths now replaced. Not a current release/STOP proof. |

## Previously integrated canonical areas

The old `feat/rt1-*` branches introduce workspace, publication, admission,
visitor-scoped conversation and signaling. Current `main` has those modules
and later versions. The November `fix/rt1-*` and `integrate/rt1-*`
branches cover worker compensation, bounded HTTP, revoked egress, STOP
reconciliation, browser lifecycle and tests: those paths are present and have
been revised by later PRs. **This audit is not a claim of formal GPU/browser
release acceptance**, and differing source hunks were not blindly cherry-picked.

## Regression carried forward

The new `tests/architecture/test_rt1_319_startup_dispatch_fence.py`
ensures the following on the current single path:

1. Visitor consent, publication preflight and unique session reservation
   precede provider creation.
2. Canonical owner-revoke and interrupt handles are registered before the
   canonical engine's provider-egress dispatch.
3. The diagnostic worker's STOP retry handle is registered before CREATE;
   returned sender identity is recorded before OFFER, and final authorization
   precedes SDP disclosure.
4. The existing guarded HTTP delivery fence still owns browser admission.

The adapted test ran locally against this exact `main` snapshot:
**4 tests passed**. Full GitHub CI and browser checks remain mandatory before merge.

## Merge policy

Only the adapted regression test is carried from old branches in this PR.
Do **not** merge/overwrite current RT1 runtime with stale branch source or
revive alternative worker/voice owners. Any future branch hunk must be
justified by a concrete missing user behavior and tested on fresh `main`.
