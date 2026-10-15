# RT0 LLM latency comparison — non-promoting evidence

This procedure compares **YandexGPT Lite** and **DeepSeek through ProxyAPI**
without changing release acceptance rules or inventing synthetic timings.

## Inputs and collection

For each LLM, use the **same candidate SHA, microphone, network, avatar,
voice profile and Russian utterance set**. Run at least **20 real owner voice
turns** and **5 STOP actions** with each provider. Export each browser-owned
session via `POST /api/evidence/session/export` and save all exports
**outside Git**, under `/secure/bench/llm-compare/<provider>/` or another
restricted path. Save the exact `provider-state.json` bytes used by that
session, without normalizing whitespace after collection. A Yandex run and a
ProxyAPI run must use separate state manifests and exported sessions.

For a 40-character candidate SHA (replace placeholders with actual inputs):

```sh
cargo run --locked -p vpr-evaluation --bin vpr-rt0-llm-latency -- \
  yandex 'gpt://FOLDER_ID/yandexgpt-lite/latest' CANDIDATE_SHA \
  /secure/bench/llm-compare/yandex/provider-state.json \
  /secure/bench/llm-compare/yandex/session-*.json \
  > /secure/bench/llm-compare/yandex/latency-report.json
```

Repeat with `proxyapi-deepseek` and the **exact** ProxyAPI model ID
recorded in that run's provider-state. Do not substitute guessed model names.

The report contains no transcript, API key or audio. It reports p50 and p95
(nearest-rank) for observed first meaningful LLM text, full LLM text, browser
first-audio and interruption stop latency, with sample counts and errors.

- Text first-meaningful p95: **2,500 ms**
- Browser first-audio p95: **3,000 ms**
- STOP p95: **500 ms**
- The comparison requires at least 20 observed samples for text/audio and
  five for STOP before assigning PASS/FAIL. Missing or fewer samples produce
  `INSUFFICIENT_EVIDENCE`.
- A completed voice attempt without browser `audio_started` remains a missing
  audio sample, **not** a success. Do not synthesize or backfill observations.

The tool rejects missing or mismatched provider-state, unknown LLM identity,
cross-candidate exports, stale schemas, duplicate session sequences, and
visitor sessions. It must never promote a candidate: only
`vpr-rt0-exit-evidence` with `ready=true` can close the RT0 gate.

## Still requires live access

A passing CI cannot verify live Yandex/ProxyAPI authorization or latency.
Provider-specific Authorization/Project headers and exact model IDs must be
tested with operator-provided credentials in a restricted environment;
credentials must not enter Git, artifacts, logs, or PR comments.

The adapter's Yandex and ProxyAPI settings are provisional until verified
against their actual API responses. For any failed live request, record a
redacted, typed failure, then update the exact vendor wire contract rather
than disabling validation.
