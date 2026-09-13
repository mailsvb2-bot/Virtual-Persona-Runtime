# RT0 release evidence

This directory is intentionally **not** proof that RT0 is complete.

An RT0 exit candidate must bind evidence to one exact combination of:

- Git commit;
- `RT0_RELEASE_SPEC.md` version + SHA-256;
- Golden manifest SHA-256;
- provider/model/voice/avatar representation versions, a sanitized provider-state manifest, and configuration fingerprints;
- automated unit/contract/E2E results;
- owner and non-owner conversation results;
- Golden Set and permission-suite results;
- latency distributions;
- cost evidence;
- human-evaluation rubric/results;
- known limitations.

Until those artifacts exist for the exact candidate, RT0 remains `EXPERIMENTAL`.

## Golden harness

The deterministic checker lives in `crates/vpr-evaluation`; its minimum manifest is `docs/evaluation/rt0_golden_minimum.json`. The bound evidence contract and CLI are documented in `docs/evaluation/README.md`.

Store real observation inputs outside the repository unless they are explicitly sanitized. A generated report is release evidence only when its candidate, suite digest, ReleaseSpec digest, and recomputed provider-state digest match the same exact candidate and real provider/model/representation state as the rest of the RT0 evidence bundle.

## Exit-evidence gate

The deterministic exit-evidence checker is `vpr-rt0-exit-evidence` in `crates/vpr-evaluation`. It rejects mock/synthetic origins, stale/cross-candidate bindings, failed Golden evidence, missing acceptance paths, quality regressions, unmeasured cost, privacy/permission failures, incomplete human review, and unreviewed limitations.

A checker PASS is necessary evidence hygiene, not proof that the referenced private artifacts are genuine. Keep the bound Golden report, the exact sanitized provider-state manifest, the sanitized exit manifest, and every hashed underlying artifact together for the exact candidate. The exit checker recomputes the provider-state digest and rejects provider/model/representation drift from the Golden report. `release.rt0_exit_gate` remains `NOT_IMPLEMENTED` until such real evidence exists and is reviewed.
