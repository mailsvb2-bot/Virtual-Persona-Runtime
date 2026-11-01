//! Thin correlation adapter; source-owned trace vocabulary remains in vpr-owner-lab.
pub(super) fn record(session: u64, request: u64, stage: &'static str, outcome: &'static str) {
    vpr_owner_lab::voice_trace_event(stage, outcome, Some(session), Some(request));
}
