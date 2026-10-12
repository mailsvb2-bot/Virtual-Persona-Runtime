# RT0 product-quality blockers

This is a human-facing issue register; the canonical RT0 maturity/exit gates
remain in docs/capabilities/RT0.json and docs/releases/RT0_RELEASE_SPEC.md.

## RT0-TTS-001 — Russian speech is artificial/metallic

- **State:** OPEN — HUMAN REJECTED (reported by owner on actual sessions).
- **Tracking:** GitHub #276.
- **Scope:** existing Windows SAPI Irina voice; stable intelligibility does NOT
  establish believable natural Russian speech.
- **Not a fix:** rate adjustment, persistent PowerShell host, successful PCM
  warmup, green CI or faster STT/LLM.
- **Accept only after:** rights-cleared neural voice chosen; owner and independent
  visitor human speech ratings passed; Russian names/dates/numbers/stress tested;
  first-audio, STOP/resume, private PCM, quality, permissions, A/V and cost
  measured on one exact frozen candidate. No unauthorized voice cloning.

## RT0-MEDIA-002 — full playout and A/V sync not proven

- **State:** OPEN.
- **Tracking:** GitHub #255.
- **Observation:** W3C RTP playout timestamps unavailable in exported owner
  evidence. Browser first-audio and provider video events cannot be promoted
  to full-playback/A-V proof.
- **Accept only after:** strict ReleaseSpec evidence on the real provider path.

## RT0-VISITOR-003 — independent human visitor acceptance missing

- **State:** OPEN.
- **Observation:** owner route and deterministic visitor tests are present;
  exported owner snapshot cannot establish independent human visitor.
- **Accept only after:** real separate consenting visitor speech/video session,
  scope-leakage check and reviewer provenance attestation on exact candidate.
