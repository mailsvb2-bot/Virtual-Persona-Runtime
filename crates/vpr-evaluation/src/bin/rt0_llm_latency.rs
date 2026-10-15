//! Non-promoting RT0 LLM latency comparison from sanitized session exports.
//! Usage: vpr-rt0-llm-latency <provider> <model> <candidate-sha> <session.json>...
use std::{env, fs, process};
use serde::Serialize;
use vpr_evaluation::{
    LabSessionEvidenceSnapshot, LabMediaEvidenceKind, LabVoiceAttemptStatus,
    RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA,
};

#[derive(Serialize)]
struct Metric {
    n: usize,
    p50_ms: u64,
    p95_ms: u64,
}

#[derive(Serialize)]
struct Metrics {
    text_first_meaningful: Option<Metric>,
    llm_full: Option<Metric>,
    first_audio: Option<Metric>,
    stop: Option<Metric>,
}

#[derive(Serialize)]
struct Thresholds {
    text_p95_ms: u64,
    first_audio_p95_ms: u64,
    stop_p95_ms: u64,
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
    Some(Metric { n: len, p50_ms: values[rank(50)], p95_ms: values[rank(95)] })
}

fn verdict(metric: &Option<Metric>, minimum: usize, threshold: u64) -> &'static str {
    match metric {
        Some(metric) if metric.n >= minimum && metric.p95_ms <= threshold => "PASS",
        Some(metric) if metric.n >= minimum => "FAIL",
        _ => "INSUFFICIENT_EVIDENCE",
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 5 {
        return Err("usage: vpr-rt0-llm-latency <provider> <model> <candidate-sha> <session.json>...".into());
    }
    let (provider, model, candidate) = (&args[1], &args[2], &args[3]);
    if !matches!(provider.as_str(), "yandex" | "proxyapi-deepseek" | "deepseek") {
        return Err("unsupported provider".into());
    }
    if candidate.len() != 40 || !candidate.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid candidate SHA".into());
    }
    let mut text = Vec::new();
    let mut full = Vec::new();
    let mut audio = Vec::new();
    let mut stop = Vec::new();
    let mut turns = 0_usize;
    let mut errors = 0_usize;
    let mut provider_state = None::<String>;
    for path in &args[4..] {
        let data = fs::read(path).map_err(|_| "session evidence unreadable")?;
        if data.len() > 8_000_000 { return Err("session evidence too large".into()); }
        let session: LabSessionEvidenceSnapshot = serde_json::from_slice(&data)
            .map_err(|_| "invalid session evidence schema")?;
        if session.schema_version != RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA
            || &session.candidate_sha != candidate
        { return Err("stale or cross-candidate session evidence".into()); }
        if provider_state.as_ref().is_some_and(|sha| sha != &session.provider_state_sha256) {
            return Err("mixed provider-state digests".into());
        }
        provider_state = Some(session.provider_state_sha256.clone());
        for attempt in &session.voice_attempts {
            turns += 1;
            if attempt.status != LabVoiceAttemptStatus::Completed {
                errors += 1;
                continue;
            }
            if let Some(ms) = attempt.llm_first_meaningful_millis { text.push(ms); }
            if let Some(ms) = attempt.llm_millis { full.push(ms); }
            if let Some(ms) = session.media_events.iter()
                .filter(|event| event.kind == LabMediaEvidenceKind::AudioStarted
                    && event.request_sequence == Some(attempt.request_sequence))
                .map(|event| event.elapsed_millis).min()
            { audio.push(ms); }
        }
        stop.extend(session.media_events.iter()
            .filter(|event| event.kind == LabMediaEvidenceKind::InterruptionStopped)
            .map(|event| event.elapsed_millis));
    }
    let metrics = Metrics {
        text_first_meaningful: percentile(text),
        llm_full: percentile(full),
        first_audio: percentile(audio),
        stop: percentile(stop),
    };
    let report = Report {
        schema: "vpr-llm-latency-compare-0.1", candidate_sha: candidate, provider, model, turns,
        errors, thresholds: Thresholds { text_p95_ms: 2500, first_audio_p95_ms: 3000, stop_p95_ms: 500 },
        verdict: Verdict {
            text: verdict(&metrics.text_first_meaningful, 20, 2500),
            first_audio: verdict(&metrics.first_audio, 20, 3000),
            stop: verdict(&metrics.stop, 5, 500),
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
    fn nearest_rank_and_minimum_sample_contract() {
        let values: Vec<u64> = (1..=20).collect();
        let metric = percentile(values);
        let metric = metric.as_ref().unwrap();
        assert_eq!((metric.p50_ms, metric.p95_ms), (10, 19));
        assert_eq!(verdict(&None, 20, 2500), "INSUFFICIENT_EVIDENCE");
        assert_eq!(verdict(&Some(super::Metric { n: 19, p50_ms: 1, p95_ms: 1 }), 20, 2500), "INSUFFICIENT_EVIDENCE");
        assert_eq!(verdict(&Some(super::Metric { n: 20, p50_ms: 1, p95_ms: 2501 }), 20, 2500), "FAIL");
    }
}
