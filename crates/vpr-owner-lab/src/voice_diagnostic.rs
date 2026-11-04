//! Bounded, secret-free operational trace for the canonical Owner Lab voice journey.
//! Only fixed source-defined labels may enter the trace. Emission NEVER blocks
//! voice ingress, the engine lock, provider STOP or worker completion.
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::OnceLock;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_TRACE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_QUEUED_EVENTS: usize = 256;

#[derive(Clone, Copy)]
struct TraceEvent {
    at_ms: u128,
    stage: &'static str,
    outcome: &'static str,
    session: Option<u64>,
    request: Option<u64>,
}

// The only trace writer is deliberately outside the canonical media path.
// When disk I/O stalls, try_send drops telemetry instead of delaying speech.
static WRITER: OnceLock<Option<SyncSender<TraceEvent>>> = OnceLock::new();

pub(crate) fn record(stage: &'static str, reason: &'static str) {
    event(stage, reason, None, None);
}

pub fn event(
    stage: &'static str,
    outcome: &'static str,
    session: Option<u64>,
    request: Option<u64>,
) {
    if !allowed_event(stage, outcome) {
        return;
    }
    let Ok(path) = std::env::var("VPR_OWNER_LAB_VOICE_DIAGNOSTIC_FILE") else {
        return;
    };
    if path.is_empty() {
        return;
    }
    let sender = WRITER.get_or_init(|| {
        let (sender, receiver) = sync_channel::<TraceEvent>(MAX_QUEUED_EVENTS);
        // Startup is only attempted when diagnostics are explicitly enabled.
        // The diagnostic worker owns its file exclusively within this process.
        if thread::Builder::new()
            .name("vpr-voice-diagnostic".to_owned())
            .spawn(move || {
                for entry in receiver {
                    write_event(&path, entry);
                }
            })
            .is_err()
        {
            return None;
        }
        Some(sender)
    });
    if let Some(sender) = sender {
        let at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |time| time.as_millis());
        // No send(), mutex, open(), flush(), rotation, or file metadata calls
        // are permitted on this latency-critical producer path.
        let _ = sender.try_send(TraceEvent {
            at_ms,
            stage,
            outcome,
            session,
            request,
        });
    }
}

fn write_event(path: &str, entry: TraceEvent) {
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    if file.metadata().is_ok_and(|m| m.len() >= MAX_TRACE_BYTES) {
        drop(file);
        let rotated = format!("{path}.previous");
        let _ = std::fs::remove_file(&rotated);
        if std::fs::rename(path, &rotated).is_err() {
            return;
        }
        let Ok(new_file) = OpenOptions::new().create_new(true).append(true).open(path) else {
            return;
        };
        file = new_file;
    }
    let line = serde_json::json!({
        "schema": "vpr-voice-trace-1",
        "timestamp_unix_ms": entry.at_ms,
        "process_id": std::process::id(),
        "stage": entry.stage,
        "outcome": entry.outcome,
        "session_sequence": entry.session,
        "request_sequence": entry.request,
    });
    let _ = writeln!(file, "{line}");
}

fn allowed_event(stage: &str, outcome: &str) -> bool {
    match stage {
        "voice_start" => outcome == "request_received",
        "stt_open" => matches!(
            outcome,
            "accepted"
                | "provider_auth_denied"
                | "provider_policy_denied"
                | "provider_other_failure"
                | "runtime_authority_denied"
        ),
        "voice_finish" => outcome == "processing_started",
        "voice_worker" => matches!(outcome, "failed" | "completed"),
        "stt_input" => matches!(outcome, "streaming" | "buffered" | "finished" | "failed"),
        "llm_generation" => matches!(outcome, "started" | "completed" | "failed"),
        "avatar_delivery" => matches!(outcome, "started" | "completed" | "failed"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::allowed_event;

    #[test]
    fn only_fixed_diagnostic_labels_are_allowed() {
        assert!(allowed_event("voice_start", "request_received"));
        assert!(allowed_event("stt_open", "provider_auth_denied"));
        assert!(!allowed_event("stt_open", "raw provider response"));
        assert!(!allowed_event("provider supplied stage", "accepted"));
        assert!(!allowed_event("stt_open", "api_key=secret"));
    }

    #[test]
    fn trace_schema_is_fixed_and_has_no_sensitive_fields() {
        let fields = [
            "schema",
            "timestamp_unix_ms",
            "process_id",
            "stage",
            "outcome",
            "session_sequence",
            "request_sequence",
        ];
        assert!(!fields.contains(&"api_key"));
        assert!(!fields.contains(&"transcript"));
        assert!(!fields.contains(&"audio"));
    }
}
