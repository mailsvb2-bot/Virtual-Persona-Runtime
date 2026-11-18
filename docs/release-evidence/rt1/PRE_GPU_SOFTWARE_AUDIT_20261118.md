# RT1 pre-GPU software closure map — 2026-11-18

**Decision: BLOCKED — do not purchase a permanent GPU server.** This document is
a code/test inspection record, **not** a release certificate or CUDA acceptance.

Baseline inspected: `main@44a590520638dc5c8da31dc90437ddbc7e7b4d6c`.
Remediation: PR #434, with a new full architecture test gate and RT1
button/API contract. Its final merge and exact-head CI outcome must be checked
separately; a PR alone is not evidence of completion.

## Verified in isolated non-production checkout

- `python3 -m unittest discover -s tests/architecture -p 'test_*.py' -q`:
  baseline 75/77 passed, **two outdated RT0 bootstrap test assumptions failed**.
  Updated test matches the current deferred watchdog/single app entrypoint,
  without restoring speculative modulepreload requests. On integration branch:
  **79/79 passed**, including a new RT1 button and route structural regression.
- `npm ci --ignore-scripts --no-audit --no-fund && npm run build`: passed
  (TypeScript and vendored LiveKit assets) in isolation.
- `python3 tests/architecture/check_browser_contract.py`: passed.
- Python RT1 focused tests: model/asset digests (4 + 6), environment (5),
  offline preflight (8), GPU acceptance metric **unit** tests (3),
  worker lifecycle (34), STOP reconciler (11), bounded CREATE (7):
  **78 tests passed** without real CUDA or network provider.
- `python3 -m compileall -q scripts tests/e2e`, `node --check
  crates/vpr-owner-lab/ui/rt1.js`, `bash -n
  scripts/rt1_musetalk_hardware_acceptance.sh`: passed.
- The full `tests/e2e/test_rt1_*.py` discovery attempted 174 tests;
  **12 errors** were missing `numpy`/`aiortc` on the isolated audit
  machine, not established runtime failures. This invocation is NOT PASS.
  Dedicated GitHub Actions with actual transport dependencies are required.
- RT1 owner/visitor markup contains 12 buttons; each has a click-handler
  registration in `ui/rt1.js`; literal RT1 API paths used there are registered
  in `http_rt1.rs`. This is structural wiring evidence, **not** a live journey.

## P0: mandatory release/real-world evidence (NOT CLOSED)

| Blocker | Canonical location | Required proof |
| --- | --- | --- |
| Formal RT0 exit is not verified | `scripts/rt1_entry_readiness.py`, `docs/releases/RT0_RELEASE_SPEC.md` | Exact-candidate RT0 exit evaluator and reviewed evidence; never remove the release fence |
| True GPU inference | `scripts/rt1_musetalk_renderer.py`, `scripts/rt1_musetalk_hardware_acceptance.sh` | Approved/licensed avatar and pinned source/weights, CUDA execution, real frames, timing evidence |
| Real browser A/V sync | `scripts/rt1_aiortc_backend.py`, `ui/rt1.js` | Actual WebRTC browser capture/playback with real model video, measured offset and recovery |
| Independent audio + video STOP | `scripts/rt1_media_worker_lifecycle.py`, `rt1_remote_media_revocation.rs` | Sender receipts and real transport observation on interruption, revocation, shutdown and restart |
| Production authorization/consent | `rt1_workspace_service.rs`, `rt1_owner_controller.rs` | Reviewed rights, identity/version and public entry gate before production publish |

## P1: pre-GPU hardening and deployment evidence

- Install a reproducible, pinned **GPU-specific** Python/CUDA/Whisper/VAE/MuseTalk
  environment in an isolated build image. Do not treat CPU CI or a dependency
  list as a CUDA compatibility certificate.
- Complete full-source-to-browser owner/visitor E2E with provider credentials
  in an approved non-production environment; track external failures separately.
- Verify exact-HEAD Rust tests, Windows credentials, browser E2E, RT1 sender
  and dependency security status after the integration PR reaches green.
- Prove clean-install, health/restart, secrets and rollback with immutable
  version/image and no mutation of a live business server.

## Gates, not estimates

**Software ready for GPU acceptance:** no known *software* P0, reproducible
offline/bootstrap/test environment, exact-HEAD CI and dedicated RTP/browser
workflows green, installation recipe reviewed, remaining GPU-only items named.

**Production ready:** additionally genuine licensed face frames and TTS PCM,
measured lip-sync/latency, dual-track RTP stop, rights/consent, RT0 release
exit, multi-session resource/cost tests, and owner/visitor human acceptance.

**No automatic promotion:** passing a fixture or an AI-generated readiness
document must never update RT0/RT1 release maturity.
