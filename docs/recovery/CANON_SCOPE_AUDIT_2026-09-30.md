# Canon v3.4 scope / conflict audit — 2026-09-30

## Conclusion

No direct normative contradiction was found that requires RT0 to implement RT1 before RT0 can finish.

Canon v3.4 explicitly separates long-term vision, architectural contract and current release scope. It also states that future capability must not become a current-release blocker unless explicitly promoted. The Release Train section reinforces that the capability map is not permission to build everything at once.

The current recovery interpretation is therefore:

> RT0 remains the active executable product path. Later-train work may exist only as genuinely bounded, non-production feasibility work and must not destabilize, enlarge, or become a dependency of the RT0 gate.

## Important tension: bounded later spikes vs. “improve core before expanding”

Two valid Canon rules need an operational tie-breaker:

1. a later Release Train MAY begin with a bounded feasibility spike when needed to remove major technical uncertainty;
2. if the RT0 exit gate fails, the project improves the core experience before expanding the platform.

These statements are compatible only when “bounded feasibility spike” remains genuinely bounded.

For recovery, a later-train spike is considered bounded only if all are true:

- it does not modify the active RT0 runtime/user path unless the change is also independently required by RT0;
- it does not add a new mandatory gate to RT0;
- it does not become a dependency of RT0 build/startup/session/media behavior;
- it does not create a second canonical authority, persistence path or provider identity;
- it does not make the active RT0 CI materially more fragile;
- it can be parked/removed without changing the RT0 user-visible result.

If those conditions are not met, the work is expansion rather than a bounded spike and waits until the RT0 core is stable.

## Repository finding

The current `main` runs a large set of RT1 pre-entry contract/spike guards while RT0 exact-candidate exit remains open.

Most of those artifacts are contract-only and are valuable design work. Their existence is not itself a Canon violation.

However, their quantity in the active mainline increases review/CI blast radius and makes Release Train priority less obvious. During recovery they remain preserved in Git history but are not re-admitted merely because they were previously merged.

## RT0 ReleaseSpec finding

The current RT0 ReleaseSpec is not substantially larger than the stable pre-sizing version. The meaningful changes are narrow, including a more explicit A/V-sync evidence contract and an adversarial visitor prompt-injection case.

Therefore the current red CI is not explained by a recent large expansion of RT0 requirements.

## A/V evidence caution

The ReleaseSpec currently specifies an exact browser/WebRTC A/V-sync evidence mechanism, including `estimatedPlayoutTimestamp` sampling.

This can be a valid exact-candidate RT0 evidence mechanism for the selected provider/transport path, but it must remain a ReleaseSpec implementation/evidence decision rather than becoming a universal Persona architecture invariant.

Provider neutrality means another supported transport may require a different, explicitly versioned evidence mechanism while still satisfying the Canon's A/V-sync quality requirement.

## Test architecture finding

The current `main` production Owner Lab app contains browser-test orchestration hooks (`__vprTestMediaRuntime`, `notifyTestApiResponse`, test bootstrap state/events) and normal API completion awaits a test callback.

That crosses the production/test boundary and makes renderer/test scheduling capable of blocking production control flow. It matches the observed nondeterministic media E2E failures.

The recovery branch therefore forbids these hooks in production `app.ts` via an architecture check. Test media fixtures must live behind a composition boundary and must never be awaited by normal production HTTP/bootstrap completion.

## Recovery rule

The recovery branch re-admits later work by independent vertical slice:

1. exact-head branch CI green;
2. architecture/release contracts green;
3. Windows path green when affected;
4. browser/backend/voice/media path green when affected;
5. real RT0 operator/live-provider proof where required;
6. only then admit the next slice.

No timeout increase, assertion weakening, or deletion of safety/contracts is accepted as a green-build fix.
