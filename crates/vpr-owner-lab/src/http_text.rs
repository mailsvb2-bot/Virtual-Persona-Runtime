use std::sync::atomic::Ordering;

use tiny_http::Request;

use crate::http_json::SpeakBody;

use super::{
    AppState, HttpResponse, error_response, http_evidence, json_response, lab_error_response,
    parse_json,
};

pub(super) fn text_turn_response(
    request: &mut Request,
    state: &AppState,
) -> Result<HttpResponse, HttpResponse> {
    // The browser already sends the session generation. Bind this text
    // request to it so an old tab cannot talk in a newly started session.
    let session_sequence = http_evidence::require_active_voice_session(request, state)?;
    if state
        .voice_busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(error_response(409, "INVALID_STATE_TRANSITION"));
    }
    let mut busy =
        http_evidence::VoiceBusyGuard::new(&state.voice_busy, &state.voice_cancel_requested);
    http_evidence::require_voice_session_sequence(state, session_sequence)?;
    *state.replay_source.lock() = None;
    let request_sequence = http_evidence::request_sequence(request)
        .map_err(|error| error_response(http_evidence::error_status(error), error.code()))?;
    let body = parse_json::<SpeakBody>(request)?;
    state
        .evidence
        .lock()
        .begin_text_request(request_sequence)
        .map_err(|error| error_response(http_evidence::error_status(error), error.code()))?;

    // Keep the engine mutex until evidence has reached a terminal state.
    // Otherwise Close can acquire engine.lock(), seal/export the session,
    // and leave the completed text attempt Pending in its evidence snapshot.
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

    // Serialize the final authority decision with the canonical Close/fence
    // transition, which also owns session_revocation. The session generation
    // must still match even when Close already reset session_end_requested.
    let _revocation = state.session_revocation.lock();
    let current = http_evidence::require_voice_session_sequence(state, session_sequence);
    if let Err(response) = current {
        state
            .evidence
            .lock()
            .fail_text_request(request_sequence, "TURN_CANCELLED")
            .map_err(|error| error_response(http_evidence::error_status(error), error.code()))?;
        // Release before dropping engine.lock(): an old guard must never
        // clear a newer turn's busy bit after Close/restart.
        busy.release();
        return Err(response);
    }
    let outcome = match result {
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
    };
    // A new turn may acquire the gate only after this attempt is terminal.
    // Keep the engine and revocation guards held until the old busy owner
    // has explicitly released its flag.
    busy.release();
    outcome
}
