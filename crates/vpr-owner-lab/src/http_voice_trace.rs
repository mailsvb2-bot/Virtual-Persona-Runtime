//! Canonical HTTP voice correlation shim. No payloads or credentials.
fn record(session: u64, request: u64, stage: &'static str, outcome: &'static str) {
    vpr_owner_lab::voice_trace_event(stage, outcome, Some(session), Some(request));
}

pub(super) fn start(session: u64, request: u64) {
    record(session, request, "voice_start", "request_received");
}

pub(super) fn stt_accepted(session: u64, request: u64) {
    record(session, request, "stt_open", "accepted");
}

pub(super) fn processing(session: u64, request: u64) {
    record(session, request, "voice_finish", "processing_started");
}

pub(super) fn failed(session: u64, request: u64) {
    record(session, request, "voice_worker", "failed");
}

pub(super) fn completed(session: u64, request: u64) {
    record(session, request, "voice_worker", "completed");
}
