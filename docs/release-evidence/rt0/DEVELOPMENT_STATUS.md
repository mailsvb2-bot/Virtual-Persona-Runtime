# RT0 Development Status

This file records development evidence only. It is **not** RT0 exit evidence.

## Experimental canonical foundation present in code

- Persona identity/version/mode skeleton and owner-opinion attribution boundary;
- authority intersection, egress policy, authorization epoch invalidation and session-scoped policy enforcement;
- linearized provider execution permits and revoke/replace/policy-change cancellation propagation;
- realtime session/turn lifecycle with explicit denied/failed/cancelled terminal paths;
- provider-neutral STT/LLM/TTS/avatar ports plus OpenAI-compatible, Anthropic, Gemini, OpenAI Transcription, Deepgram, OpenAI Speech, ElevenLabs and D-ID adapter contracts;
- guided owner interview and modality-readiness foundations;
- loopback Owner Lab API for minimal Persona creation, guided capture, explicit claim approval/correction and initial review completion, backed by the canonical `vpr-capture` state machine;
- single-owner transfer of the reviewed `PersonaProfile` from capture into the Owner Lab runtime rather than a second copied Persona state;
- canonical media timeline, output-delivery evidence and interruption/reconnect contracts;
- loopback Owner Lab WebRTC avatar path and push-to-talk STT -> LLM -> avatar conversation path;
- owner-reviewed DIGITAL_TWIN context binding for Owner Lab voice turns, including correction-driven PersonaVersion changes and next-turn use of the corrected current claim revision;
- deterministic Golden harness and exact-candidate RT0 exit-evidence checker;
- credentialed live-proof preflight and sanitized live-provider reachability probe;
- sanitized Owner Lab session-evidence capture and deterministic multi-session latency/cost aggregation;
- exact-candidate + exact provider-state binding for aggregated Owner Lab session evidence;
- stable RT0 reason-code vocabulary and capability maturity guards.

## Still open for RT0 exit

RT0 is not complete. Real credentialed owner and non-owner conversations still need to be captured and reviewed for one exact candidate. The remaining exit evidence includes browser UI for the guided capture/review/correction flow, its E2E proof, real provider proof for the supported STT/LLM/avatar path, complete text/audio/video/A-V-sync latency evidence, measured duration/cost, owner interruption, Golden observations from the real candidate, permission/privacy acceptance, human voice/appearance/persona/conversation evaluation, known-limitations review, and the full owner/visitor happy/correction/failure/recovery/revoke-deny acceptance matrix.

The owner-capture API and reviewed-owner-context path remain `EXPERIMENTAL` and `user_reachable=false` under the RT0 maturity ceiling. The loopback API is an implementation surface, not evidence that the owner-facing product journey is complete; UI and E2E evidence are still required.

The bound session aggregate is evidence preparation only. It preserves `canonical_playback_proven=false` and `av_sync_proven=false` and MUST NOT by itself promote `provider.real_*`, `evaluation.rt0_golden_set`, or `release.rt0_exit_gate` beyond their current catalogue state.

The capability catalogue remains the machine-readable maturity source.
