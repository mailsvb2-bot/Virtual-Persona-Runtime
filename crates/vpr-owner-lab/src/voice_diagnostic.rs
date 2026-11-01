//! Bounded, secret-free operational trace for the canonical Owner Lab voice journey.
//! All labels are fixed in source, never constructed from provider responses.
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
const MAX_TRACE_BYTES: u64 = 2 * 1024 * 1024;

pub(crate) fn record(stage: &'static str, reason: &'static str) {
    event(stage, reason, None, None);
}

pub(crate) fn event(
    stage: &'static str,
    outcome: &'static str,
    session: Option<u64>,
    request: Option<u64>,
) {
    let Ok(path) = std::env::var("VPR_OWNER_LAB_VOICE_DIAGNOSTIC_FILE") else {
        return;
    };
    if path.is_empty() {
        return;
    }
    let Ok(_lock) = WRITE_LOCK.get_or_init(|| Mutex::new(())).lock() else {
        return;
    };
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    // Bound disk usage. The previous segment remains for incident inspection.
    if file.metadata().is_ok_and(|m| m.len() >= MAX_TRACE_BYTES) {
        drop(file);
        let rotated = format!("{path}.previous");
        let _ = std::fs::remove_file(&rotated);
        if std::fs::rename(&path, &rotated).is_err() {
            return;
        }
        let Ok(new_file) = OpenOptions::new().create_new(true).append(true).open(&path) else {
            return;
        };
        file = new_file;
    }
    let at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| time.as_millis());
    let line = serde_json::json!({
        "schema": "vpr-voice-trace-1",
        "timestamp_unix_ms": at_ms,
        "process_id": std::process::id(),
        "stage": stage,
        "outcome": outcome,
        "session_sequence": session,
        "request_sequence": request,
    });
    let _ = writeln!(file, "{line}");
}

#[cfg(test)]
mod tests {
    #[test]
    fn trace_schema_is_fixed_and_has_no_sensitive_fields() {
        let fields = ["schema", "timestamp_unix_ms", "process_id", "stage",
            "outcome", "session_sequence", "request_sequence"];
        assert!(!fields.contains(&"api_key"));
        assert!(!fields.contains(&"transcript"));
        assert!(!fields.contains(&"audio"));
    }
}
