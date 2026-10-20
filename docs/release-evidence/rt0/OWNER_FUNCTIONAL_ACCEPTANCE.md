# R0 — owner functional acceptance (20 October 2026)

**Owner decision: R0 ACCEPTED / CLOSED for the agreed functional demonstration.**

Accepted candidate: `c393bae4cb369c82ef3f9160b9ba5fc6dc2d5774`.

The owner explicitly accepts the working end-to-end demonstration:
1. Start Owner Lab with one Windows CMD command.
2. Connect the realtime avatar, audio and video.
3. Ask a question in Russian; the avatar starts speaking its answer.
4. Interrupt speaking using the STOP control.
5. Resume speaking using the continuation control.
6. End the session intentionally before the entire response finishes, to avoid unnecessary real-provider charges.
7. Run owner and visitor sessions on the same frozen candidate.

Evidence: real owner session 1 and visitor session 2, schema `rt0-owner-lab-session-evidence-1.6`, both bound to the candidate above and the same provider-state SHA-256 `ebdbb91de77b3786fd4c21c100fde27e9793fc7e839f4d34fb849997136ba16b`. Both record a completed voice request, `audio_started`, `interruption_stopped`, and connected video. Human operator confirms the interrupt/resume user experience. Because full speech was intentionally interrupted, no `playback_completed` receipt was expected for these sessions. Raw private evidence is intentionally not committed.

**Scope and explicit distinction:** This is owner sign-off on a functional R0 demonstration and closure of that milestone. It is **not** a claim that the canonical `vpr-rt0-exit-evidence` release gate passed, that `PRODUCTION_READY` is true, that full playout or strict A/V synchronization is proven, or that the interim Piper voice is cleared for commercial use. Those remain open follow-up acceptance/release tasks and must not be silently converted into green verdicts.

## Deferred work (post owner-accepted R0)

- Provider-authoritative full playback receipt and three strict WebRTC A/V sync samples on an **uninterrupted** response.
- Human naturalness, persona, appearance, and voice-rights/commercial licensing review.
- Real correction/recovery/revoke/deny and owner/visitor provenance/privacy acceptance paths.
- Golden Set, full latency distributions, p95 targets, provider cost evidence, known limitations and exact-bound exit evidence inventory.
- Canonical RT0 release verifier remains fail-closed pending these artifacts.

Do not reclassify intentionally interrupted cost-saving test sessions as a speech defect. Do not spend live-provider budget merely to duplicate an already owner-accepted STOP/resume demonstration.
