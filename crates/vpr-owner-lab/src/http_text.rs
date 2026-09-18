use std::sync::atomic::Ordering;

use tiny_http::Request;

use super::{
    AppState, HttpResponse, SpeakBody, error_response, http_evidence, json_response,
    lab_error_response, parse_json, reject_if_session_ending,
};

pub(super) fn text_turn_response(
    request: &mut Request,
    state: &AppState,
) -> Result<HttpResponse, HttpResponse> {
    reject_if_session_ending(state)?;
    if state
        .voice_busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(error_response(409, "INVALID_STATE_TRANSITION"));
    }
    let _busy =
        http_evidence::VoiceBusyGuard::new(&state.voice_busy, &state.voice_cancel_requested);
    reject_if_session_ending(state)?;
    let request_sequence = http_evidence::request_sequence(request)
        .map_err(|error| error_response(http_evidence::error_status(error), error.code()))?;
    let body = parse_json::<SpeakBody>(request)?;
    state
        .evidence
        .lock()
        .begin_text_request(request_sequence)
        .map_err(|error| error_response(http_evidence::error_status(error), error.code()))?;

    let result = {
        let Ok(mut engine) = state.engine.lock() else {
            let _ = state
                .evidence
                .lock()
                .fail_text_request(request_sequence, "INTERNAL_ERROR");
            return Err(error_response(500, "INTERNAL_ERROR"));
        };
        let result = engine.text_turn(&body.text, |handle| {
            *state.active_voice_interrupt.lock() = Some(handle.clone());
            if state.voice_cancel_requested.load(Ordering::Acquire) {
                let _ = handle.interrupt();
            }
        });
        *state.active_voice_interrupt.lock() = None;
        result
    };
    match result {
        Ok(value) => state
            .evidence
            .lock()
            .complete_text_request(request_sequence, &value)
            .map(|()| json_response(200, &value))
            .map_err(|error| error_response(http_evidence::error_status(error), error.code())),
        Err(error) => {
            if let Err(evidence_error) = state
                .evidence
                .lock()
                .fail_text_request(request_sequence, error.code())
            {
                Err(error_response(
                    http_evidence::error_status(evidence_error),
                    evidence_error.code(),
                ))
            } else {
                Err(lab_error_response(&error))
            }
        }
    }
}
