# RT0 visitor journey vs RT1 visitor admission — source trace (2026-11-02)

This is a source-level trace, **not** a live D-ID audio/video STOP certificate.

## Existing canonical RT0 journey (main and PR #324)

- `crates/vpr-owner-lab/ui/src/app.ts`: visitor selects `audience: "visitor"` and POSTs `/api/avatar/start` with `consent: true`. Browser connects the returned WebRTC/LiveKit transport. The same UI records PCM from `getUserMedia` and uploads via `/api/voice/input/start`, `/api/voice/input/chunk`, `/api/voice/input/finish`; it polls `/api/voice/events`, sends `/api/avatar/interrupt`, and fences/revokes/closes via `/api/session/fence`, `/api/session/revoke`, `/api/session/close`.
- `crates/vpr-owner-lab/src/main.rs`: `/api/avatar/start` selects `OwnerLabEngine::start_visitor`, registers the active session with `http_session::register_started_session`, and binds the evidence session sequence. `http_voice.rs` uploads PCM to the **same** canonical `OwnerLabEngine`, using `begin_voice_input` and `finish_voice_input_streaming`. `state/voice.rs` executes canonical STT -> LLM -> TTS/avatar delivery; `state/voice_delivery.rs` calls the runtime-owned delivery APIs.
- `http_interrupt.rs` uses the existing independent server-side stop handle without waiting for the engine mutex; `http_session.rs` revokes the runtime authority epoch before cleanup and fences stale session generations.
- `ui/e2e/backend-voice-journey.spec.ts` and `ui/e2e/voice-journey-driver.js` are byte-identical between main and PR #324 (Git blob SHA `ac2e656d2ae4567b40f8d723bb0fee3786fa9aac` and `bfbd2b9d9f48e4bfd2869f3729212ed910009439`). Their test installs `fake-webrtc-media-runtime.js` and provider fixtures. It proves the tested browser/backend contract, **not** remote D-ID RTP sender shutdown.

## RT1 entry (PR #324)

- `http_rt1.rs` serves an explicitly local experimental `/rt1` entry and `/api/rt1/visitor/*`. A visitor token is checked against the durable publication via `Rt1OwnerController::visitor_sessions()`. Each visitor gets a **distinct** `OwnerLabEngine` assembled by the same `engine_bootstrap::assemble`, and `restore_reviewed_persona` checks the reviewed Persona. The owner RT0 engine/CSRF are not used as visitor publication authority.
- `rt1_visitor_conversation.rs` calls the **existing** `OwnerLabEngine::start_visitor` and `OwnerLabEngine::voice_turn`, rather than a second STT/LLM/TTS implementation. It stages microphone PCM through `VisitorSessionManager`, registers canonical interrupt/STOP handles, and checks authority before/after a turn. `text_turn` and `voice_turn_streaming` remain denied because the final revocation-serialized provider egress fence required by #319 is not yet proven. Never replace these guards with simple pre/post checks.
- `rt1.js` captures browser microphone audio and POSTs `/api/rt1/visitor/voice`, but this is not equivalent to the RT0 streaming upload, interruption, and evidence contract. Its LiveKit and WebRTC UI tests use fixtures.
- `rt1_remote_media_revocation.rs` and the durable worker STOP ledger are distinct from the legacy D-ID provider DELETE. The legacy provider close can succeed while **remote video RTP termination remains unproven**.

## Security correction in this change

Previously `http_rt1.rs::visitor_close` could return HTTP 200 and remove the visitor registration even though `remote_stop_confirmed: false` was in the response. The browser rejected that response but could no longer retry STOP using its session ID.

Now local-only STOP returns HTTP 502 `RT1_REMOTE_VIDEO_STOP_UNCONFIRMED`, retains the session lookup for retry, and keeps publication revoked. Only the independent worker path that verifies **both** audio and video sender receipts may return a confirmed close. The backend-backed Playwright regression in `rt1-canonical-provider.spec.ts` checks the 502, retained revoked session and denied microphone voice. Its D-ID endpoint is a fixture, not the real provider.

## Release blockers

1. A final, revocation-serialized provider egress/STOP boundary for RT1 text and streaming microphone turns; do not borrow the RT0 owner HTTP session as RT1 visitor authority.
2. A real browser microphone -> STT -> LLM -> TTS -> avatar audio/video -> interrupt/revoke E2E on **RT1** with a production-like, authorized provider. Current browser fixture tests are insufficient.
3. Durable correlation of an unknown provider-generated CREATE session ID after lost response, and independent evidence of both remote D-ID RTP sender stops after revoke/failure/restart.
4. Full exact-HEAD CI, dependency security and RTP probe after each patch, followed by independent D-ID staging review before removing Draft or merging.
