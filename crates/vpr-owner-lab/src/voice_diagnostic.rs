//! Fixed-vocabulary operational diagnostics; no customer speech, tokens or provider bodies.
use std::fs::OpenOptions;
use std::io::Write;

pub(crate) fn record(stage: &'static str, reason: &'static str) {
    let Ok(path) = std::env::var("VPR_OWNER_LAB_VOICE_DIAGNOSTIC_FILE") else {
        return;
    };
    if path.is_empty() {
        return;
    }
    // Caller supplies only hard-coded labels. Never serialize provider errors.
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "VPR_VOICE stage={stage} reason={reason}");
    }
}
