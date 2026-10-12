# RT0 — operator handoff from live owner/visitor sessions

**Non-promoting diagnostic only.** The RT0 exit verdict remains exclusively in
the canonical vpr-rt0-exit-evidence evaluator and its exact-bound evidence bundle.

## Workflow

1. Freeze the exact RT0 candidate and provider configuration.
2. Conduct a **real owner** session using the canonical browser. Stop speech,
   finish and export its terminal sanitized session JSON.
3. Conduct a **distinct, consenting non-owner human** visitor session with the
   same exact candidate/provider state. Export its terminal JSON separately.
4. Record actual human Russian speech quality as unreviewed, rejected or accepted.
   This flag is **input from a reviewer**, never inferred from speech timing.
5. From the repository root run:

    python3 scripts/rt0_live_acceptance.py --speech-quality rejected <owner-session.json>
    python3 scripts/rt0_live_acceptance.py --speech-quality rejected --format json <owner-session.json> <visitor-session.json>

The command accepts bounded schema 1.6 browser session snapshots only. It rejects
mixed candidates, mixed provider-state digests, unknown roles, malformed metrics,
duplicates and unsupported scopes, prints only a sanitized subset, and never
exports transcripts, Persona claims or raw biometric media.

## What this proves — and does not

A completed voice-turn event is a browser-observed owner or visitor operation,
not independent human identity verification. An observed STOP latency (e.g.
85 ms) is not a p95 quality-conformance claim. First audible browser event
is not full provider playback. A/V sync remains unproven if the browser does
not provide the strict W3C estimatedPlayoutTimestamp samples. Known STT/LLM
times cannot establish full avatar/provider cost.

The output is always tagged NOT_RELEASE_EVIDENCE and
release_ready_claimed=false, even with owner and visitor snapshots and an
accepted human speech-quality input. The operator must still run the
RT0_RUNBOOK canonical artifact inventory, credentialed conversations,
owner Golden, reviewed human/permissions/cost evidence, supporting capture,
exact-snapshot aggregate and exit verifier. Do not copy diagnostic states
into canonical supporting-evidence files.

## Next accepted work

- P1 Russian voice-quality defect tracked in GitHub issue #276; Windows Irina
  is not human-accepted. No deployment of unlicensed actress reference voice.
- Obtain independent human visitor session and provenance attestation.
- Prove provider-confirmed full playout and strict A/V synchronization.
- Measure/review actual provider cost and all remaining QualityContract paths.
- Complete correction, recovery, revoke/deny, Golden, privacy and human review.

## Release-train safety

RT1 ReleaseSpec remains BLOCKED_ON_RT0_EXIT and allows bounded feasibility
spikes only. This operator workflow is RT0 diagnostics; it creates no RT1
publication endpoint, account, second Persona authority or durable state.

## Executable correction -> new response -> visitor safety E2E

The browser + real Rust Owner Lab HTTP/provider-fixture E2E now carries three
sessions, not two isolated tests. Original owner text/voice/interruption and
recovery are recorded under the initial reviewed Persona. After terminal
evidence export the owner corrects one exact opinion; canonical persistence
advances PersonaVersion. A newly opened owner session sends real HTTP-backed
text and microphone/STT -> LLM -> avatar voice turns. The E2E inspects the
**provider-received LLM request bodies** to ensure that only the NEW reviewed
revision appears in the new turns while the original turns preserve the old
revision. Then a visitor requests text and voice without either private
revision; mutation/read of owner claims is denied. After revoke, a new
protected text turn is denied and the fixture confirms zero new provider egress.

The deterministic provider fixture proves system behavior, not a distinct
human visitor or real D-ID/LLM quality. RT0 exit remains blocked on the real
human/provider evidence matrix.
