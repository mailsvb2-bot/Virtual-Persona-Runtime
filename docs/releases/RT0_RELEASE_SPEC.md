# RT0 ReleaseSpec — Feasibility & Wow Proof

**Spec version:** `RT0-0.1.2`
**Status:** ACTIVE  
**Normative parent:** `docs/CANON.md` v3.4  
**Maturity ceiling during RT0:** `EXPERIMENTAL` until the RT0 exit gate is evidenced.

## 1. Goal

Prove, with real providers and measured evidence, that a real owner can create a believable Russian-speaking `DIGITAL_TWIN`, hold a voice/video conversation with it, interrupt it, and allow at least one non-owner test participant to do the same without false owner-opinion attribution or private-context leakage.

RT0 is a feasibility and quality proof. It is not the first production release and MUST NOT claim `PRODUCTION_READY`.

## 2. Entry condition

- Canon v3.4 is present in-repository as the single normative source.
- The project is a new Git history, not a fork or subtree continuation.
- No provider-specific identity is treated as canonical Persona, Voice or Appearance identity.
- This ReleaseSpec is committed before the RT0 runtime implementation begins.

## 3. Exact user journey

1. Owner opens the RT0 lab and creates a minimal Persona in `DIGITAL_TWIN` mode.
2. Owner completes a short guided capture/interview and explicitly approves the initial identity/attribution boundary.
3. Owner connects or records the minimum voice/appearance references required by the chosen RT0 providers.
4. Preparation reports independent readiness for text, voice and video; a failed video preparation MUST NOT destroy Persona state.
5. Owner starts a test session and speaks in Russian.
6. Microphone input is transcribed, the canonical runtime authorizes the turn, a real LLM produces a response, TTS produces speech, and the chosen avatar path produces visual output.
7. Owner can interrupt speech; cancellation propagates through the active turn path and unplayed tail content is not recorded as spoken.
8. Owner sees measured latency and estimated/observed provider cost for the test session.
9. Owner can correct a captured owner claim; the next test turn uses the corrected reviewed state.
10. A non-owner test participant enters a visitor-scoped test session and converses with the same Persona under visitor permissions.
11. Revoking the active test authorization prevents a new sensitive provider call or owner voice/appearance use.
12. The exact candidate is evaluated against the Golden Set, permission suite and human-review rubric.

## 4. Explicit non-goals

RT0 does not implement public marketplace, general memory, autonomous workflows, Persona teams, full portability, general-purpose research, billing, arbitrary character generation, multi-tenant production administration, or production publication by public link.

No RT0 shortcut may create a second canonical brain, provider-owned Persona state, or silent external-provider fallback that violates egress policy.

## 5. Canonical RT0 domain

Minimum durable/logical concepts:

- `PersonaId`
- `PersonaVersion`
- `PersonaMode::DigitalTwin`
- `OwnerClaim` with orthogonal claim/source/verification/derivation dimensions
- `ConstitutionBoundary`
- `AuthorizationEpoch`
- `SessionId`
- `TurnId`
- `CorrelationId`
- `TurnExecutionSnapshot`
- provider-neutral Voice and Appearance representation bindings
- modality readiness: `NOT_READY | PREPARING | READY | FAILED`

Provider IDs are bindings only and MUST NOT become any canonical identity key.

## 6. State machines

### Persona test readiness

`DRAFT -> CAPTURED -> REVIEWED -> TEST_READY`

A Persona may be `TEST_READY` when text is ready even while voice/video are independently preparing; UI must show partial readiness.

### Preparation job

`QUEUED -> PREPARING -> VALIDATING -> READY | FAILED | CANCELLED`

Provider failure is recoverable by retry/replacement without recreating Persona identity or reviewed claims.

### Realtime session

`CREATED -> ACTIVE -> DRAINING -> CLOSED`

Exceptional transition: `CREATED|ACTIVE|DRAINING -> REVOKED -> CLOSED`.

### Turn

`RECEIVED -> AUTHORIZED -> PROCESSING -> OUTPUTTING -> COMPLETED`

Failure/cancellation states: `DENIED | FAILED | CANCELLED`.

A cancelled turn MUST preserve evidence that distinguishes generated, sent, played and cancelled output.

## 7. Provider-neutral ports

RT0 defines canonical ports for:

- `SttPort`
- `LlmPort`
- `TtsPort`
- `AvatarPort`

Vendor SDK types MUST NOT cross into `vpr-domain`. Adapters declare provider/model/representation versions and return structured provider evidence, measured usage/cost inputs and typed failures.

Initial provider selection is an implementation decision recorded in RT0 evidence, not canonical identity. The credentialed RT0 provider-state manifest binds distinct `STT`, `LLM`, `TTS` and `Avatar` roles. An avatar provider's built-in speech does not substitute for proof of the canonical `TtsPort`; the live-provider reachability probe must exercise `TtsPort` through the runtime boundary when RT0 evidence is collected.

## 8. Authorization and egress

Effective authority is intersection-based across applicable policy layers. Explicit deny, revocation and expiry win.

Before every external LLM/STT/TTS/avatar call, RT0 must evaluate at least:

- data classification;
- active authorization epoch/lease;
- consent for real-person voice/appearance where applicable;
- provider/region/data-policy compatibility;
- requested purpose;
- hard local-only/deny constraints.

Canonical outcomes: `ALLOW | REDACT | LOCAL_ONLY | DENY`.

There is no external fallback from `LOCAL_ONLY` or `DENY`.

## 9. Owner-attribution boundary

For `DIGITAL_TWIN`, an answer MUST NOT present an opinion as a verified owner opinion unless the supporting claim is explicitly eligible under the reviewed owner-attribution rule.

At minimum, `MODEL` source, `INFERRED` derivation, `SIMULATED` derivation, or `UNVERIFIED` verification is ineligible for verified-owner attribution.

The runtime must preserve a machine-testable distinction between verified owner material, inference, simulation and unknown content.

## 10. API / realtime contract for RT0

The RT0 owner UI will use a thin versioned API surface. Exact transport payloads may evolve inside RT0 before the first exit-gate candidate, but semantic contracts and reason codes below are fixed by this spec.

Required operations:

- create minimal digital-twin Persona;
- submit guided capture answers;
- review/correct an owner claim;
- start/stop preparation;
- query per-modality readiness;
- start/revoke/close a test session;
- send/stream audio into a session;
- receive transcript, text response, audio/video events and turn lifecycle events;
- interrupt an active turn;
- query session telemetry/cost evidence.

Realtime transport must preserve canonical ordering/correlation IDs and expose interruption flush semantics.

## 11. Reason codes

Minimum stable reason-code families:

- `AUTH_REVOKED`
- `AUTH_EXPIRED`
- `AUTH_SCOPE_DENIED`
- `EGRESS_DENIED`
- `EGRESS_LOCAL_ONLY`
- `CONSENT_REQUIRED`
- `PROVIDER_UNAVAILABLE`
- `PROVIDER_RATE_LIMITED`
- `PROVIDER_TIMEOUT`
- `PREPARATION_FAILED`
- `TURN_CANCELLED`
- `INVALID_STATE_TRANSITION`
- `OWNER_ATTRIBUTION_UNVERIFIED`
- `BUDGET_EXHAUSTED`
- `INTERNAL_ERROR`

Unknown failures fail closed when they touch sensitive identity, biometric use or external egress.

## 12. Failure and recovery

- LLM/TTS/STT/avatar provider failure must surface a typed reason and preserve the session where safe.
- Video failure may degrade to voice/text only when policy permits and the user is informed.
- Lost provider acknowledgement that could have produced a side effect/charge is represented as uncertain evidence and reconciled before blind retry.
- Reconnect must not create a new Persona identity or silently widen authority.
- Owner corrections and revocations must win over stale/cached state.

## 13. QualityContract RT0-0.1.2

Provisional engineering targets from the Canon are adopted unchanged:

- text first meaningful response: p50 <= 1.0 s, p95 <= 2.5 s;
- first meaningful audio: p50 <= 1.5 s, p95 <= 3.0 s;
- interruption stop after detection: p95 <= 0.5 s;
- prepared-avatar first useful video after media path ready: p95 <= 2.5 s;
- A/V sync absolute offset: p95 <= 120 ms;
- recoverable realtime reconnect: p95 <= 5 s;
- accepted private-context leakage: 0;
- accepted false attribution of unverified owner opinion: 0.

RT0 calibrates these with real measurements. They may not be silently weakened.

For the RT0 browser WebRTC path, the A/V synchronization reference is the W3C WebRTC Stats `RTCInboundRtpStreamStats.estimatedPlayoutTimestamp` pair for the single active inbound audio and video tracks of the same avatar session. A sample is the absolute difference, in milliseconds, between the audio and video estimated playout timestamps, collected only after canonical playback has been confirmed for the exact voice request. Exactly three samples, sequence-numbered `1..=3`, are required for every completed canonical-playback voice request before that request contributes A/V-sync proof. If either timestamp is unavailable, non-finite, no packets have been received, or the browser exposes an ambiguous set of inbound audio/video tracks, that sample is unavailable and MUST NOT be replaced by wall-clock guessing, first-frame timing, RTP-clock subtraction, or provider-specific timestamps. Human appearance/conversation review remains separate from this media-plane synchronization measurement.

## 14. Telemetry and cost

Each material turn gets a correlation ID. Trace evidence must distinguish, when present: VAD, STT, context assembly, LLM TTFT, TTS first audio, renderer first frame, transport and interruption.

Raw secrets, private documents, raw prompts, raw audio/video and biometric material are not logged by default.

Cost evidence distinguishes estimate, measured usage and provider charge where the provider exposes them. Missing confirmed provider charge must not be invented.

## 15. Acceptance matrix

### Happy path

- Owner completes capture, test readiness and a real Russian voice/video conversation.
- Non-owner participant completes a visitor-scoped real conversation.
- Exact-candidate latency/cost evidence is produced.

### User correction path

- Owner corrects a captured claim.
- A subsequent turn uses the corrected revision.
- Previous derived/test evidence remains attributable to the earlier revision rather than being silently rewritten.

### Failure + recovery path

- One active provider fails or times out.
- Runtime emits a typed reason, preserves canonical Persona/session state and either safely retries/replaces/degrades according to policy or stops the affected modality.
- Conversation can recover without identity drift where the selected provider stack supports recovery.

### Revoke / deny path

- Active authorization/consent is revoked or egress is denied.
- A new protected provider call is blocked.
- Runtime exposes the reason without leaking protected context.

## 16. Golden and adversarial minimum

The RT0 suite must include Russian-language cases for names/surnames, dates, numbers, abbreviations, natural phrasing and domain terms, plus:

- inferred owner opinion must not become verified owner opinion;
- simulated response must not become owner fact/opinion;
- private owner context is unavailable to visitor role;
- revoked authority blocks sensitive use;
- cancelled/unplayed output is not persisted as spoken;
- provider failure does not mutate Persona identity.

## 17. Human evaluation

For the exit-gate candidate, human review records at least:

- voice similarity;
- voice naturalness;
- appearance plausibility;
- Persona similarity;
- conversation naturalness.

The rubric/version and results are bound to the exact candidate and provider/model/representation versions.

## 18. Release evidence

`docs/release-evidence/rt0/` will bind:

- exact Git commit;
- this ReleaseSpec digest/version;
- automated test results;
- E2E results;
- provider/model/representation versions;
- latency measurements;
- cost measurements;
- permission/privacy results;
- human evaluation;
- known limitations.

Evidence from a materially different code/configuration/provider state is stale for the exit decision.

## 19. Rollback and migration

Before RT0 exit, persistence is explicitly experimental. Every schema change that touches canonical identity, reviewed owner claims, authorization, consent or execution evidence must either be forward-migrated or the experimental data must be intentionally discarded with documented reset semantics.

Provider replacement never changes `PersonaId`.

## 20. Exit gate

RT0 is complete only when all Canon RT0 exit conditions are evidenced on one exact candidate. If any mandatory quality/privacy/attribution criterion fails, RT0 remains open and platform expansion waits.
