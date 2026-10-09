//! Provider STOP must not wait behind an STT/LLM worker's engine mutex.
use std::sync::atomic::Ordering;

use serde::Deserialize;
use vpr_owner_lab::LabError;

use super::{
    AppState, HttpResponse, OwnerLabTurnInput, error_response, http_session::reject_if_session_ending,
    json_response, lab_error_response, with_engine_result,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InterruptBody {
    #[serde(default)]
    expected_session_sequence: Option<u64>,
}

pub(super) fn interrupt_response(
    request: &mut tiny_http::Request,
    state: &AppState,
) -> Result<HttpResponse, HttpResponse> {
    let body: InterruptBody = super::parse_json(request)?;
    reject_if_session_ending(state)?;
    let server_stop = state.backend_stop.lock().clone();
    if let Some(stop) = server_stop {
        // A stale tab must never STOP an unrelated newer room. Echo's sender is
        // session-private, but the HTTP request must still name its generation.
        if body.expected_session_sequence != Some(state.active_session_sequence.load(Ordering::Acquire))
            || body.expected_session_sequence == Some(0)
        {
            return Err(error_response(409, "INVALID_STATE_TRANSITION"));
        }
        if state.voice_busy.load(Ordering::Acquire) {
            state.voice_cancel_requested.store(true, Ordering::Release);
            if let Some(turn) = state.active_voice_interrupt.lock().clone() {
                turn.interrupt().map_err(|reason| lab_error_response(&LabError::Runtime(reason)))?;
            }
        }
        // No engine.lock(): the server LiveKit sender receives STOP immediately
        // even if an LLM request or speech turn is currently generating.
        stop.stop().map_err(|error| lab_error_response(&LabError::Provider(error.reason_code())))?;
        return Ok(json_response(200, &serde_json::json!({"ok": true})));
    }
    if state.voice_busy.load(Ordering::Acquire) {
        state.voice_cancel_requested.store(true, Ordering::Release);
        if let Some(handle) = state.active_voice_interrupt.lock().clone() {
            return handle
                .interrupt()
                .map(|()| json_response(200, &serde_json::json!({"ok": true})))
                .map_err(|reason| lab_error_response(&LabError::Runtime(reason)));
        }
        return Ok(json_response(200, &serde_json::json!({"ok": true})));
    }
    with_engine_result(state, |engine| {
        engine
            .apply(OwnerLabTurnInput::Interrupt)
            .map(|()| json_response(200, &serde_json::json!({"ok": true})))
    })
}
