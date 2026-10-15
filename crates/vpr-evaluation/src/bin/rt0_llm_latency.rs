//! Non-promoting RT0 LLM latency comparison from sanitized session exports.
//! Usage: vpr-rt0-llm-latency <provider> <model> <candidate-sha> <provider-state.json> <session.json>...
use serde::Serialize;
use std::{collections::HashSet, env, fs, process};
use vpr_evaluation::{
    LabMediaEvidenceKind, LabSessionEvidenceSnapshot, LabVoiceAttemptStatus, ParticipantRole,
    ProviderRole, ProviderStateManifest, RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA, sha256_hex,
    validate_provider_state_manifest,
};

#[derive(Serialize)]
struct Metric {
    n: usize,
    p50_ms: u64,
    p95_ms: u64,
}

#[derive(Serialize)]
struct Metrics {
    /// LLM call-relative first meaningful chunk, not microphone-to-first-text latency.
    llm_first_meaningful: Option<Metric>,
    text_first_meaningful: Option<Metric>,
    llm_full: Option<Metric>,
    /// Session-relative event positions, NOT response latency.
    browser_audio_event_offset: Option<Metric>,
    /// Session-relative STOP event positions, NOT STOP latency.
    stop_event_offset: Option<Metric>,
    first_audio: Option<Metric>,
    stop: Option<Metric>,
}

#[derive(Serialize)]
struct Thresholds {
    #[serde(rename = "text_p95_ms")]
    text: u64,
    #[serde(rename = "first_audio_p95_ms")]
    first_audio: u64,
    #[serde(rename = "stop_p95_ms")]
    stop: u64,
}

#[derive(Serialize)]
struct Verdict {
    text: &'static str,
    first_audio: &'static str,
    stop: &'static str,
}

#[derive(Serialize)]
struct Report<'a> {
    schema: &'static str,
    candidate_sha: &'a str,
    provider: &'a str,
    model: &'a str,
    turns: usize,
    errors: usize,
    measurement_note: &'static str,
    metrics: Metrics,
    thresholds: Thresholds,
    verdict: Verdict,
}

fn percentile(mut values: Vec<u64>) -> Option<Metric> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let len = values.len();
    let rank = |pct: usize| len.saturating_mul(pct).div_ceil(100).max(1) - 1;
    Some(Metric {
        n: len,
        p50_ms: values[rank(50)],
        p95_ms: values[rank(95)],
    })
}

fn verdict(metric: Option<&Metric>, minimum: usize, threshold: u64) -> &'static str {
    match metric {
        Some(metric) if metric.n >= minimum && metric.p95_ms <= threshold => "PASS",
        Some(metric) if metric.n >= minimum => "FAIL",
        _ => "INSUFFICIENT_EVIDENCE",
    }
}

#[derive(Default)]
struct Samples {
    text: Vec<u64>,
    full: Vec<u64>,
    audio: Vec<u64>,
    stop: Vec<u64>,
    turns: usize,
    errors: usize,
}

fn collect_samples(
    paths: &[String],
    candidate: &str,
    state_digest: &str,
) -> Result<Samples, String> {
    let mut samples = Samples::default();
    let mut session_ids = HashSet::new();
    for path in paths {
        let data = fs::read(path).map_err(|_| "session evidence unreadable")?;
        if data.len() > 8_000_000 {
            return Err("session evidence too large".into());
        }
        let session: LabSessionEvidenceSnapshot =
            serde_json::from_slice(&data).map_err(|_| "invalid session evidence schema")?;
        if session.schema_version != RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA
            || session.candidate_sha != candidate
        {
            return Err("stale or cross-candidate session evidence".into());
        }
        if session.participant_role != ParticipantRole::Owner {
            return Err("benchmark requires owner session evidence".into());
        }
        if !session_ids.insert(session.session_sequence) {
            return Err("duplicate session sequence".into());
        }
        if session.provider_state_sha256 != state_digest {
            return Err("session provider-state digest mismatch".into());
        }
        for attempt in &session.voice_attempts {
            samples.turns += 1;
            if attempt.status != LabVoiceAttemptStatus::Completed {
                samples.errors += 1;
                continue;
            }
            if let Some(ms) = attempt.llm_first_meaningful_millis {
                samples.text.push(ms);
            }
            if let Some(ms) = attempt.llm_millis {
                samples.full.push(ms);
            }
            if let Some(ms) = session
                .media_events
                .iter()
                .filter(|event| {
                    event.kind == LabMediaEvidenceKind::AudioStarted
                        && event.request_sequence == Some(attempt.request_sequence)
                })
                .map(|event| event.elapsed_millis)
                .min()
            {
                samples.audio.push(ms);
            }
        }
        samples.stop.extend(
            session
                .media_events
                .iter()
                .filter(|event| event.kind == LabMediaEvidenceKind::InterruptionStopped)
                .map(|event| event.elapsed_millis),
        );
    }
    Ok(samples)
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 6 {
        return Err(
            "usage: vpr-rt0-llm-latency <provider> <model> <candidate-sha> <provider-state.json> <session.json>..."
                .into(),
        );
    }
    let (provider, model, candidate) = (&args[1], &args[2], &args[3]);
    if !matches!(
        provider.as_str(),
        "yandex" | "proxyapi-deepseek" | "deepseek"
    ) {
        return Err("unsupported provider".into());
    }
    if candidate.len() != 40 || !candidate.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid candidate SHA".into());
    }
    let state_bytes = fs::read(&args[4]).map_err(|_| "provider state unreadable")?;
    if state_bytes.len() > 1_000_000 {
        return Err("provider state too large".into());
    }
    let state: ProviderStateManifest =
        serde_json::from_slice(&state_bytes).map_err(|_| "invalid provider state")?;
    validate_provider_state_manifest(&state).map_err(|_| "unsupported provider state")?;
    let binding = state
        .providers
        .iter()
        .find(|entry| entry.role == ProviderRole::Llm)
        .ok_or("LLM provider binding missing")?;
    if &binding.provider != provider || &binding.model_or_representation != model {
        return Err("provider or model differs from canonical provider state".into());
    }
    let state_digest = sha256_hex(&state_bytes);
    let samples = collect_samples(&args[5..], candidate, &state_digest)?;
    let metrics = Metrics {
        llm_first_meaningful: percentile(samples.text),
        text_first_meaningful: None,
        llm_full: percentile(samples.full),
        browser_audio_event_offset: percentile(samples.audio),
        stop_event_offset: percentile(samples.stop),
        first_audio: None,
        stop: None,
    };
    let report = Report {
        schema: "vpr-llm-latency-compare-0.3",
        candidate_sha: candidate,
        provider,
        model,
        turns: samples.turns,
        errors: samples.errors,
        measurement_note: "LLM first meaningful time is relative to LLM start, not microphone input; media events are session-relative positions. The end-to-end text/audio/STOP p95 verdicts require aligned start timestamps and remain INSUFFICIENT_EVIDENCE.",
        thresholds: Thresholds {
            text: 2500,
            first_audio: 3000,
            stop: 500,
        },
        verdict: Verdict {
            text: verdict(metrics.text_first_meaningful.as_ref(), 20, 2500),
            first_audio: verdict(metrics.first_audio.as_ref(), 20, 3000),
            stop: verdict(metrics.stop.as_ref(), 5, 500),
        },
        metrics,
    };
    let json = serde_json::to_string_pretty(&report).map_err(|_| "report serialization failed")?;
    println!("{json}");
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("vpr-rt0-llm-latency: {error}");
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{percentile, verdict};
    #[test]
    fn session_relative_media_offsets_are_never_accepted_as_latency() {
        let metrics = super::Metrics {
            llm_first_meaningful: percentile(vec![1; 20]),
            text_first_meaningful: None,
            llm_full: None,
            browser_audio_event_offset: percentile(vec![1; 20]),
            stop_event_offset: percentile(vec![1; 5]),
            first_audio: None,
            stop: None,
        };
        assert_eq!(
            verdict(metrics.text_first_meaningful.as_ref(), 20, 2500),
            "INSUFFICIENT_EVIDENCE"
        );
        assert_eq!(metrics.llm_first_meaningful.unwrap().n, 20);
        assert_eq!(
            verdict(metrics.first_audio.as_ref(), 20, 3000),
            "INSUFFICIENT_EVIDENCE"
        );
        assert_eq!(
            verdict(metrics.stop.as_ref(), 5, 500),
            "INSUFFICIENT_EVIDENCE"
        );
        assert_eq!(metrics.browser_audio_event_offset.unwrap().n, 20);
    }

    #[test]
    fn nearest_rank_and_minimum_sample_contract() {
        let values: Vec<u64> = (1..=20).collect();
        let metric = percentile(values);
        let metric = metric.as_ref().unwrap();
        assert_eq!((metric.p50_ms, metric.p95_ms), (10, 19));
        assert_eq!(verdict(None, 20, 2500), "INSUFFICIENT_EVIDENCE");
        assert_eq!(
            verdict(
                Some(&super::Metric {
                    n: 19,
                    p50_ms: 1,
                    p95_ms: 1
                }),
                20,
                2500
            ),
            "INSUFFICIENT_EVIDENCE"
        );
        assert_eq!(
            verdict(
                Some(&super::Metric {
                    n: 20,
                    p50_ms: 1,
                    p95_ms: 2501
                }),
                20,
                2500
            ),
            "FAIL"
        );
    }
}
