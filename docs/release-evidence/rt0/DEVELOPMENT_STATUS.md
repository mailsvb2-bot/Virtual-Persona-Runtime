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
- credentialed live-proof preflight and sanitized STT/LLM/TTS/avatar live-provider reachability probe; TTS is exercised through canonical `ActiveTurn::execute_tts` with its own sanitized provider/model fingerprint, synthesized-audio digest/duration and usage evidence without being misrepresented as part of the STT/LLM/realtime-avatar conversation provider state;
- credentialed owner/visitor conversation-attempt runner that reuses the canonical Owner Lab engine and emits only sanitized exact-candidate/provider-state-bound receipts;
- atomic credentialed candidate runner that prevalidates private inputs before egress, runs provider reachability plus owner/visitor attempt under one frozen candidate, rejects provider-state drift between stages, and publishes no partial evidence artifacts;
- sanitized Owner Lab session-evidence capture and deterministic multi-session latency/cost aggregation;
- CSRF-protected exact-byte export of terminal sanitized Owner Lab session snapshots, with a server-side next-session gate so a closed/revoked owner or visitor snapshot cannot be silently replaced before an explicit export request; the server-side gate remains authoritative across browser reloads while the independent export action remains available;
- server-derived owner/visitor participant-role binding in raw Owner Lab session evidence, with exit/inventory recomputation of Russian, completed-turn, canonical-playback/voice, rendered-video and interruption conversation claims from the credentialed receipt plus exact raw snapshots;
- exact-candidate + exact provider-state binding for aggregated Owner Lab session evidence;
- evidence-inventory `0.4` verification of exit-manifest candidate/provider and presented artifact digests, reuse of the canonical supporting-artifact validator, plus recomputation of the exact raw sanitized session-snapshot binding, rejecting missing, stale, tampered, duplicate, or extra/unbound evidence;
- exit-gate recomputation of bound Owner Lab session aggregates from the exact raw sanitized snapshots, including fail-closed canonical playback proof and exact binding of browser-derived first-audio, interruption-stop, first-video, reconnect and A/V-sync QualityEvidence distributions before threshold evaluation;
- release-CLI binding of all ten declared supporting-evidence digests to the exact presented artifact bytes before any `ready=true` decision, plus canonical supporting-projection equality for the nine JSON artifacts; CI/E2E projections bind `candidate_sha`, while the seven real conversation/acceptance/quality/cost/privacy/human projections bind both `candidate_sha` and `provider_state_sha256`, preventing stale cross-candidate/provider reuse through rehashing;
- non-promoting typed supporting-evidence preflight for all ten canonical supporting artifacts, including exact candidate/provider binding, real-origin enforcement, owner/visitor role checks, latency-shape validation, mandatory five-dimension human-review completeness and artifact digest output without any release-ready claim;
- exact-byte binding of the known-limitations review decision: `known-limitations.md` carries an explicit `RT0-Review-Status: passed|failed` marker that both preflight and the exit verifier parse and require to match the manifest review status, preventing an independent manual status flip after rehashing;
- non-promoting exact-artifact exit-manifest assembler that reuses canonical supporting/probe/conversation/session validators, derives all supporting claims and digests from presented bytes, preserves failed outcomes unchanged, and publishes a new manifest atomically without overwriting an existing one;
- request-scoped browser A/V sync sampling from W3C WebRTC `estimatedPlayoutTimestamp`, requiring three sequence-numbered samples after canonical playback for each completed voice request;
- runtime-issued avatar output segments with backend-only browser playback reconciliation and sanitized per-attempt canonical playback evidence;
- stable RT0 reason-code vocabulary and capability maturity guards;
- browser-contract E2E plus real Owner Lab HTTP/runtime/provider-adapter E2E for owner/visitor sessions and STT -> LLM -> avatar voice turns, including post-response D-ID Fluent client-side interruption, disconnected -> connected WebRTC recovery with sanitized reconnect-restored evidence, and an OpenAI-compatible LLM 503 -> typed PROVIDER_UNAVAILABLE -> successful next-turn recovery in the same reviewed owner Persona/session, using deterministic test-only provider endpoints.

## Still open for RT0 exit

RT0 is not complete. The repository now has a fail-closed credentialed owner/visitor conversation-attempt path, but a real attempt still needs to be executed on one exact candidate and paired with browser media-plane and human review evidence. The guided capture/review/correction browser flow and owner/visitor voice path now have deterministic development E2E through the real Owner Lab HTTP/runtime and adapter implementations. The remaining exit evidence is credentialed real-provider owner and non-owner conversation proof on one exact candidate, complete text/audio/video/A-V-sync and recoverable-reconnect latency evidence, measured duration/cost, real owner interruption on that exact candidate, Golden observations from that real candidate, permission/privacy acceptance, human voice/appearance/persona/conversation evaluation, known-limitations review, and the full real owner/visitor happy/correction/failure/recovery/revoke-deny acceptance matrix. D-ID interruption, reconnect recovery, and provider failure/recovery mechanisms are now covered only by deterministic development E2E and do not substitute for those real exact-candidate observations.

The owner-capture API, browser controls, post-review correction path, reviewed-owner-context path, and visitor-scoped test-session path remain `EXPERIMENTAL` and `user_reachable=false` under the RT0 maturity ceiling. The loopback implementation plus deterministic browser E2E are development evidence only; credentialed live-provider and human evidence are still required before promotion.

The exit checker requires raw sanitized Owner Lab session snapshots and recomputes their exact candidate/provider-state-bound aggregate before accepting runtime-backed evidence. Schema `0.5` adds payload-redacted canonical text attempts with runtime turn/output receipts and backend-observed first meaningful LLM response timing, while request-scoped A/V sync still derives only from three explicit WebRTC estimated-playout samples per completed canonical-playback voice turn, without provider-clock or wall-clock fallback. The exit checker now requires recomputed text-first-response, first-audio, interruption-stop, first-video, reconnect and A/V-sync distributions to exactly equal their corresponding `QualityEvidence` fields; a detached or hand-edited browser-derived metric fails structurally. Neither playback nor A/V sampling by itself MUST promote `provider.real_*`, `evaluation.rt0_golden_set`, or `release.rt0_exit_gate` beyond their current catalogue state because the remaining real-provider, privacy, Golden, cost and human evidence is still mandatory.

The capability catalogue remains the machine-readable maturity source.
