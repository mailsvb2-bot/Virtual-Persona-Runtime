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
