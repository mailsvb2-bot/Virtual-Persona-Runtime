# RT0 owner closure — Windows

The guided script `scripts/rt0-owner-closure.ps1` executes **Compare**, **Exit**, and **Finish** without pretending that automated tests are real owner/visitor evidence.

## Compare

Run `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\rt0-owner-closure.ps1 -Stage Compare`.

For **YandexGPT 5 Lite** and **DeepSeek via ProxyAPI**: configure the credential in Windows Credential Manager, probe the live LLM wire variant (without exporting keys), launch the strict RT0 Owner Lab with a pinned interim Piper voice, perform at least 20 Russian voice turns and five real STOP operations for each LLM, then export the browser evidence. The script checks the exact candidate/provider-state and produces `latency-report.json` with measured first text, first audio, and STOP p50/p95. A probe succeeds only after an actual HTTP response and never fabricates an LLM response.

## Exit and Finish

Run `-Stage Exit -ExitLlm yandex` or `-ExitLlm proxyapi-deepseek`. Use a **real owner and a different real visitor**. Review private Golden, safety, attribution, naturalness, consent, cost and the known limitations according to `RUNBOOK.md`. Then run `-Stage Finish` with genuine reviewed inputs.

RT0 with the interim **Irina** voice is development-only: the underlying dataset rights are not cleared for commercial release. Piper's own GPL obligations need legal review. Neither voice may become `ACCEPTED` automatically.

The default evidence directory is `%LOCALAPPDATA%\Virtual-Persona-Runtime\rt0-evidence\<candidate SHA>`, outside the Git worktree. Send only sanitized `latency-report.json`, `llm-wire-probe.json`, `runtime-readiness.json`, `inventory.json`, `rt0-exit-report.json`, and `closure-summary.json`. Never send keys, raw speech or unredacted snapshots.

A green CI does not replace human evidence or prove real audible playback.
