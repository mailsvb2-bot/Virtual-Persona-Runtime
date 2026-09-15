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
- loopback Owner Lab browser controls for minimal Persona creation, guided capture, explicit claim approval/correction and initial review, with realtime Connect blocked until reviewed owner context exists;
- owner-only post-review browser correction over a CSRF-protected current-revision snapshot; corrections preserve history internally, advance PersonaVersion, and the next canonical turn reads the corrected current revision;
- visitor-scoped Owner Lab sessions over the same reviewed Persona identity/version, with owner-reviewed claim text and claim-count metadata withheld from visitor scope, direct speech injection denied, and visitor LLM context assembled without owner-private material;
- deterministic Golden harness and exact-candidate RT0 exit-evidence checker;
- credentialed live-proof preflight and sanitized live-provider reachability probe;
- credentialed owner/visitor conversation-attempt runner that reuses the canonical Owner Lab engine and emits only sanitized exact-candidate/provider-state-bound receipts;
- sanitized Owner Lab session-evidence capture and deterministic multi-session latency/cost aggregation;
- exact-candidate + exact provider-state binding for aggregated Owner Lab session evidence;
- exit-gate recomputation of bound Owner Lab session aggregates from the exact raw sanitized snapshots, including fail-closed canonical playback proof;
- runtime-issued avatar output segments with backend-only browser playback reconciliation and sanitized per-attempt canonical playback evidence;
- stable RT0 reason-code vocabulary and capability maturity guards;
- browser-contract E2E plus real Owner Lab HTTP/runtime/provider-adapter E2E for owner/visitor sessions and STT -> LLM -> avatar voice turns, using deterministic test-only provider endpoints.

## Still open for RT0 exit

RT0 is not complete. The repository now has a fail-closed credentialed owner/visitor conversation-attempt path, but a real attempt still needs to be executed on one exact candidate and paired with browser media-plane and human review evidence. The guided capture/review/correction browser flow and owner/visitor voice path now have deterministic development E2E through the real Owner Lab HTTP/runtime and adapter implementations. The remaining exit evidence is credentialed real-provider owner and non-owner conversation proof on one exact candidate, complete text/audio/video/A-V-sync latency evidence, measured duration/cost, owner interruption, Golden observations from that real candidate, permission/privacy acceptance, human voice/appearance/persona/conversation evaluation, known-limitations review, and the full real owner/visitor happy/correction/failure/recovery/revoke-deny acceptance matrix.

The owner-capture API, browser controls, post-review correction path, reviewed-owner-context path, and visitor-scoped test-session path remain `EXPERIMENTAL` and `user_reachable=false` under the RT0 maturity ceiling. The loopback implementation plus deterministic browser E2E are development evidence only; credentialed live-provider and human evidence are still required before promotion.

The exit checker now requires raw sanitized Owner Lab session snapshots and recomputes their exact candidate/provider-state-bound aggregate before accepting runtime-backed `canonical_playback_proven=true`. A forged, rehashed, stale or cross-input aggregate fails structurally. `av_sync_proven` remains false, and playback evidence by itself MUST NOT promote `provider.real_*`, `evaluation.rt0_golden_set`, or `release.rt0_exit_gate` beyond their current catalogue state.

The capability catalogue remains the machine-readable maturity source.
