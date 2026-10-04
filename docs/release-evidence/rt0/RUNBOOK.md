# RT0 Exact-Candidate Exit Runbook

This is the canonical operator order for attempting the RT0 exit gate. It is a procedure for
collecting and validating evidence; it is **not** evidence itself and it never authorizes a maturity
promotion.

The governing rule is simple: one clean Git candidate, one exact sanitized provider state, one
external evidence workspace, and no hand-edited canonical generated artifacts.

## 0. Freeze the exact candidate

Run from a clean checkout of the commit that is actually being evaluated.

```bash
git status --porcelain
git rev-parse HEAD
```

Record the exact SHA as `CANDIDATE`. Do not continue with a dirty worktree.

The exact candidate must have successful repository automation for the required CI/E2E jobs. Keep
the GitHub Actions run identifiers or other reviewed automation references outside the repository;
they are used later by the automation capture input.

**Invalidation rule:** if code changes, the candidate SHA changes, or the selected provider/model/
representation configuration changes, do not mix old and new evidence. Start again from this step
for the new candidate/provider state.

## 1. Prepare the external evidence workspace

Evidence must live outside the Git worktree.

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-evidence-prepare -- \
  /secure/evidence/rt0-candidate
```

The command verifies the clean candidate and exact compiled ReleaseSpec and creates the external
workspace without manufacturing provider, browser, Golden, cost, privacy or human evidence.

**STOP:** any preparation error must be fixed before provider egress.

## 2. Run the no-egress doctor

Validate private inputs and provider configuration without spending provider calls.

```bash
cargo run -p vpr-live-proof -- doctor \
  /secure/input/probe.raw \
  /secure/input/reviewed-profile.json \
  /secure/input/owner.raw \
  /secure/input/visitor.raw
```

**STOP:** do not run credentialed capture unless doctor passes on the exact candidate.

## 3. Capture one credentialed candidate bundle

Prefer the single-file candidate capture so provider state cannot drift between the reachability
probe and owner/visitor attempt.

```bash
VPR_LIVE_PROOF_ALLOW_EGRESS=true cargo run -p vpr-live-proof -- candidate-bundle \
  /secure/input/probe.raw \
  /secure/input/reviewed-profile.json \
  /secure/input/owner.raw \
  /secure/input/visitor.raw \
  /secure/evidence/rt0-candidate/candidate-bundle.json
```

Project the canonical downstream artifacts while still on the same exact candidate:

```bash
cargo run -p vpr-live-proof -- candidate-bundle-extract \
  /secure/evidence/rt0-candidate/candidate-bundle.json \
  /secure/evidence/rt0-candidate/provider-state.json \
  /secure/evidence/rt0-candidate/provider-probe.json \
  /secure/evidence/rt0-candidate/conversation-attempt.json
```

A successful bundle proves credentialed provider reachability and canonical owner/visitor policy
execution only. It does not prove browser playback, video, A/V sync, participant identity, human
quality or release readiness.

## 4. Create the non-promoting supporting scaffold

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-supporting-scaffold -- \
  /secure/evidence/rt0-candidate/supporting \
  /secure/evidence/rt0-candidate/provider-state.json \
  "$CANDIDATE"
```

Every substantive value starts failed, synthetic or missing. Do not edit the canonical scaffold files by hand.

## 5. Capture real browser owner and distinct non-owner sessions

Start Owner Lab with explicit egress permission:

```bash
cargo run -p vpr-owner-lab -- --allow-egress
```

On the same candidate/provider state, collect a real owner session and a session with a
**distinct real non-owner human**. The required live observations include Russian conversation, audible
canonical playback, rendered video, real owner interruption, A/V-sync samples and a recoverable
reconnect observation.

Every exported release-evidence snapshot is stamped by Owner Lab with the exact clean Git candidate
and provider-state digest captured at process start. Never hand-add or rewrite those provenance fields;
the binder rejects missing, stale, cross-candidate or cross-provider snapshots.

Terminate or revoke each session and export its exact sanitized evidence snapshot before starting
the next session. Save the exact response bytes outside the worktree, for example:

```text
/secure/evidence/rt0-candidate/session-owner.json
/secure/evidence/rt0-candidate/session-visitor.json
```

The server-side `participant_role` proves policy scope only. It does not prove human identity; that
is separately reviewed in the human-evaluation evidence.

## 6. Bind the raw session snapshots

Use the exact provider-state bytes and candidate SHA:

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-session-aggregate -- \
  bind \
  /secure/evidence/rt0-candidate/provider-state.json \
  "$CANDIDATE" \
  /secure/evidence/rt0-candidate/session-owner.json \
  /secure/evidence/rt0-candidate/session-visitor.json \
  > /secure/evidence/rt0-candidate/bound-session-aggregate.json
```

When using shell redirection, write to a temporary external file first and publish it only after the
command exits successfully; never preserve a truncated file from a failed command.

## 7. Run runtime-readiness diagnostics before another paid run

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-runtime-readiness -- \
  /secure/evidence/rt0-candidate/provider-state.json \
  /secure/evidence/rt0-candidate/conversation-attempt.json \
  /secure/evidence/rt0-candidate/bound-session-aggregate.json \
  "$CANDIDATE" \
  /secure/evidence/rt0-candidate/session-owner.json \
  /secure/evidence/rt0-candidate/session-visitor.json
```

Exit `0` means only the exact-bound runtime conversation/quality portion is complete and inside the
canonical RT0 quality thresholds. The report still states `release_ready_claimed=false`.

Exit `1` is actionable evidence, not success. If an observation is missing, repeat the necessary
browser capture without changing candidate/provider state. If a threshold failure requires a code,
model, provider or configuration change, the exact-candidate evidence is invalidated: return to
step 0.

Exit `2` means malformed, stale, cross-candidate, cross-provider or detached input. Do not
continue.

## 8. Project runtime-backed supporting evidence

Only after readiness exit `0`:

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-runtime-supporting -- \
  /secure/evidence/rt0-candidate/supporting \
  /secure/evidence/rt0-candidate/provider-state.json \
  /secure/evidence/rt0-candidate/conversation-attempt.json \
  /secure/evidence/rt0-candidate/bound-session-aggregate.json \
  "$CANDIDATE" \
  /secure/evidence/rt0-candidate/session-owner.json \
  /secure/evidence/rt0-candidate/session-visitor.json
```

This replaces only the untouched runtime scaffold entries for owner conversation, visitor
conversation and quality.

## 9. Capture reviewed CI/E2E evidence

Create a private review input outside the worktree. The references identify the exact automation
runs reviewed for this candidate. Raw references stay private; their SHA-256 digests are retained in
the canonical CI/E2E supporting files and are also repeated in the capture receipt, so the later
supporting-artifact digest remains bound to the reviewed automation provenance.

```json
{
  "schema_version": "rt0-automation-observations-0.1",
  "candidate_sha": "<exact candidate SHA>",
  "attestation": "reviewed_exact_candidate_automation",
  "ci": {
    "status": "passed",
    "evidence_reference": "github-actions:CI:<run-id>"
  },
  "e2e": {
    "status": "passed",
    "evidence_reference": "github-actions:E2E:<run-id>"
  }
}
```

Then replace only the untouched CI/E2E scaffold entries:

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-automation-capture -- \
  /secure/input/rt0-reviewed-automation.json \
  /secure/evidence/rt0-candidate/supporting \
  "$CANDIDATE"
```

Do not use `passed` unless the referenced automation for this exact candidate was actually
reviewed and successful. Failed automation may be recorded as failed; capture does not promote it.

## 10. Prefill, review and capture the manual evidence

Generate the private review input from runtime-authoritative facts:

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-supporting-input-prefill -- \
  /secure/input/rt0-reviewed-observations.json \
  /secure/evidence/rt0-candidate/provider-state.json \
  /secure/evidence/rt0-candidate/bound-session-aggregate.json \
  "$CANDIDATE" \
  /secure/evidence/rt0-candidate/session-owner.json \
  /secure/evidence/rt0-candidate/session-visitor.json
```

The generated input is deliberately blocked with `REVIEW_REQUIRED`. A real reviewer must inspect
the exact-candidate observations and complete acceptance, privacy/permissions, cost coverage,
participant provenance, all human quality dimensions and known limitations. Avatar cost/coverage
must be added only from real billing or a reviewed provider-specific estimate. Change the
attestation to `reviewed_real_observations` only after that review.

Then capture the reviewed manual evidence:

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-supporting-capture -- \
  /secure/input/rt0-reviewed-observations.json \
  /secure/evidence/rt0-candidate/supporting \
  /secure/evidence/rt0-candidate/provider-state.json \
  "$CANDIDATE"
```

No tool may infer that the owner was a real human or that the visitor was a distinct real non-owner
human. Those are explicit reviewer attestations.

## 11. Preflight all supporting evidence

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-supporting-preflight -- \
  /secure/evidence/rt0-candidate/supporting \
  /secure/evidence/rt0-candidate/provider-state.json \
  "$CANDIDATE"
```

Require `preflight_complete=true`. This is structural completeness, not release readiness.

## 12. Generate the baseline and owner-specific Golden reports

Run the mandatory baseline Golden suite against the private exact-candidate observations. Capture
stdout to a temporary external file and publish it as
`/secure/evidence/rt0-candidate/bound-golden-report.json` only after exit `0`:

```bash
cargo run -p vpr-evaluation -- \
  docs/evaluation/rt0_golden_minimum.json \
  /secure/evidence/rt0-candidate/private-golden-evidence.json \
  /secure/evidence/rt0-candidate/release-spec.md \
  /secure/evidence/rt0-candidate/provider-state.json \
  "$CANDIDATE"
```

Generate the private owner-specific report:

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-owner-golden -- \
  /secure/evidence/rt0-candidate/owner-golden-suite.json \
  /secure/evidence/rt0-candidate/owner-golden-evidence.json \
  /secure/evidence/rt0-candidate/release-spec.md \
  /secure/evidence/rt0-candidate/provider-state.json \
  /secure/evidence/rt0-candidate/owner-golden-report.json \
  "$CANDIDATE"
```

Both Golden evaluations must pass. Never substitute the repository synthetic example for real
release evidence.

## 13. Assemble the exact exit manifest

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-exit-assemble -- \
  /secure/evidence/rt0-candidate/supporting \
  /secure/evidence/rt0-candidate/bound-golden-report.json \
  /secure/evidence/rt0-candidate/owner-golden-suite.json \
  /secure/evidence/rt0-candidate/owner-golden-evidence.json \
  /secure/evidence/rt0-candidate/owner-golden-report.json \
  /secure/evidence/rt0-candidate/provider-state.json \
  /secure/evidence/rt0-candidate/provider-probe.json \
  /secure/evidence/rt0-candidate/conversation-attempt.json \
  /secure/evidence/rt0-candidate/bound-session-aggregate.json \
  /secure/evidence/rt0-candidate/release-spec.md \
  /secure/evidence/rt0-candidate/exit-evidence.json \
  "$CANDIDATE"
```

The assembler is non-promoting. It derives digests and projections from the exact presented bytes
and refuses overwrite.

## 14. Require a complete inventory

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-evidence-inventory -- \
  /secure/evidence/rt0-candidate \
  "$CANDIDATE"
```

**STOP unless `inventory_complete=true`.** Inventory completeness still is not RT0 readiness.

## 15. Run the final RT0 exit gate

```bash
cargo run -p vpr-evaluation --bin vpr-rt0-exit-evidence -- \
  /secure/evidence/rt0-candidate/exit-evidence.json \
  /secure/evidence/rt0-candidate/bound-golden-report.json \
  /secure/evidence/rt0-candidate/private-golden-evidence.json \
  /secure/evidence/rt0-candidate/owner-golden-suite.json \
  /secure/evidence/rt0-candidate/owner-golden-evidence.json \
  /secure/evidence/rt0-candidate/owner-golden-report.json \
  /secure/evidence/rt0-candidate/provider-state.json \
  /secure/evidence/rt0-candidate/provider-probe.json \
  /secure/evidence/rt0-candidate/conversation-attempt.json \
  /secure/evidence/rt0-candidate/bound-session-aggregate.json \
  /secure/evidence/rt0-candidate/supporting \
  /secure/evidence/rt0-candidate/session-owner.json \
  /secure/evidence/rt0-candidate/session-visitor.json \
  /secure/evidence/rt0-candidate/release-spec.md \
  "$CANDIDATE"
```

Exit `0` plus report `ready=true` is the deterministic final gate for the presented exact evidence.
Exit `1` means the evidence is structurally valid but one or more mandatory RT0 requirements still
fail. Exit `2` means malformed, stale, tampered or cross-bound evidence.

Only after the exact archived evidence genuinely represents the real observations it claims, the
inventory is complete, and the final report is `ready=true` may Issue #216 be closed or RT0
maturity promotion be considered.
