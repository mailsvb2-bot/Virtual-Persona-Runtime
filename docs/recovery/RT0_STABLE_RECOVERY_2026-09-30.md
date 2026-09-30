# RT0 stable recovery — 2026-09-30

## Purpose

Reconstruct the active RT0 path from the last verified pre-display-sizing baseline, then selectively re-admit later work only with evidence.

This branch MUST NOT be merged into `main` as a wholesale rollback. The later history remains preserved in `main`; useful changes are to be reviewed and re-applied deliberately.

## Baseline

- Branch: `recovery/rt0-stable-baseline-20260930`
- Baseline commit: `54e7c4dee53835a00074bf61c41849f7304d5fdf`
- Baseline merge: PR #82 — `fix: allow LiveKit HTTPS signal fallback in Owner Lab`
- Historical CI run: `35650795186` — SUCCESS
- Next PR: #83 — first explicit avatar stage containment / overflow sizing change
- PR #84 then adds aspect-safe framing / `object-fit: contain`

The baseline already contains:
- provider-neutral realtime transport with LiveKit;
- the working Windows RT0 live-proof preparation from PR #79;
- secure HTTPS/WSS LiveKit signaling fallback;
- green Rust, architecture/release contract checks, browser E2E, backend E2E and backend-integrated voice E2E.

## Current-main failure signature

Current `main` failures repeatedly concentrate in browser media suites, especially `owner-lab-ui-voice-e2e`.

The failure moved across otherwise unrelated commits. In particular, PR #185 changes RT0 candidate/evidence tooling only, yet the following `main` CI failed in the voice browser suite. The observed failure is a 120s renderer/test timeout at a post-bootstrap `page.evaluate()` media fixture operation.

That is treated as evidence of a timing/orchestration coupling in the browser media test boundary, not as proof that RT0 evidence tooling itself broke voice.

PR #189 is intentionally not used as the recovery base: it is a large draft refactor from current `main` and remains red in both Voice and Expressive browser E2E.

## Re-admission policy

Later work is preserved in Git history and classified before re-entry.

### A. Rebuild/revalidate on the canonical live path

Any change touching:
- Owner Lab bootstrap;
- browser media runtime;
- microphone/media discovery;
- LiveKit/WebRTC lifecycle;
- playback/A/V evidence ordering;
- provider connect orchestration;
- realtime UI status/readiness;
- voice/video E2E fixtures.

These changes must be reintroduced in small vertical slices with production behavior first and test seams behind stable boundaries.

### B. Candidate for selective preservation after compatibility checks

Examples include:
- Windows credential persistence and restart tooling;
- Persona persistence/restart fixes;
- local/self-hosted avatar provider support;
- RT0 evidence capture, Golden-set and immutable candidate-bundle tooling;
- provider diagnostics and reason-code improvements.

Each candidate must compile and pass the full RT0 gate on this branch before the next candidate is added.

### C. Park until RT0 exit is green

RT1 pre-entry spikes/contracts merged while RT0 exit remained unfinished are preserved in `main` history but are not part of this recovery branch yet.

A later-train feasibility spike may inform design, but it must not destabilize or expand the active RT0 production path.

## Recovery gate

For every re-admitted slice:

1. architecture/release contracts;
2. format/clippy/unit tests;
3. ordinary Owner Lab browser E2E;
4. backend-integrated E2E;
5. voice browser E2E;
6. Expressive/LiveKit E2E when that capability is present;
7. Windows credential/restart tests when affected;
8. real operator/live-provider proof where required by RT0 ReleaseSpec;
9. no timeout increases or assertion weakening used to manufacture green;
10. exact-head evidence recorded before the next slice.

If a supposedly unrelated slice makes media E2E red, treat that as evidence of remaining nondeterminism and fix the boundary before proceeding.
