use std::sync::atomic::Ordering;

use vpr_domain::Rt0ReasonCode;
use vpr_owner_lab::LabError;

use super::{AppState, HttpResponse, error_response, json_response, lab_error_response};

pub(super) fn reject_if_session_ending(state: &AppState) -> Result<(), HttpResponse> {
    if state.session_end_requested.load(Ordering::Acquire) {
        Err(error_response(409, "INVALID_STATE_TRANSITION"))
    } else {
        Ok(())
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionEndBody {
    // Legacy internal callers may still send {}. Browsers must include their
    // observed session sequence to prevent stale tabs closing a newer session.
    #[serde(default)]
    expected_session_sequence: Option<u64>,
}

pub(super) fn end_session_response(
    request: &mut tiny_http::Request,
    state: &AppState,
    close: bool,
) -> Result<HttpResponse, HttpResponse> {
    let body: SessionEndBody = super::parse_json(request)?;
    end_session(state, close, body.expected_session_sequence)
}

fn request_voice_cancel(state: &AppState) {
    if !state.voice_busy.load(Ordering::Acquire) {
        return;
    }
    state.voice_cancel_requested.store(true, Ordering::Release);
    if let Some(handle) = state.active_voice_interrupt.lock().clone() {
        let _ = handle.interrupt();
    }
    let _ = super::http_voice::cancel_active_input(
        state,
        LabError::Runtime(Rt0ReasonCode::TurnCancelled),
    );
}

fn end_session(
    state: &AppState,
    close: bool,
    expected_session_sequence: Option<u64>,
) -> Result<HttpResponse, HttpResponse> {
    // Serialize conditional teardown with new-session startup using the same
    // replay_source -> engine lock order. A stale tab may NEVER terminate a
    // different (newer) session, even when its unload keepalive arrives late.
    {
        let mut replay = state.replay_source.lock();
        let engine = state
            .engine
            .lock()
            .map_err(|_| error_response(500, "INTERNAL_ERROR"))?;
        if expected_session_sequence.is_some_and(|sequence| {
            sequence == 0 || engine.current_session_sequence() != Some(sequence)
        }) {
            return Err(error_response(409, "INVALID_STATE_TRANSITION"));
        }
        if !matches!(engine.status().session_state.as_str(), "active" | "revoked") {
            return Err(error_response(409, "INVALID_STATE_TRANSITION"));
        }
        state.session_end_requested.store(true, Ordering::Release);
        *replay = None;
    }
    request_voice_cancel(state);
    // Cancel and REVOKE AUTHORITY before waiting for slow or wedged STT/LLM
    // workers. A timeout below must never leave canonical session_state active.
    // Provider cleanup and evidence sealing are separate from permission removal.
    {
        let mut engine = state
            .engine
            .lock()
            .map_err(|_| error_response(500, "INTERNAL_ERROR"))?;
        engine
            .revoke_authority()
            .map_err(|error| lab_error_response(&error))?;
        // An in-flight voice stream is not a reason to leave D-ID's remote
        // session running. Try remote cleanup under already-revoked authority,
        // BEFORE the bounded quiescence wait (which can return 504).
        engine
            .revoke()
            .map_err(|error| lab_error_response(&error))?;
    }
    if !state.voice_streams.wait_until_quiescent() {
        return Err(error_response(504, "PROVIDER_TIMEOUT"));
    }
    // A worker may have finished after the initial cache clear. Teardown is
    // authoritative: no completed response survives this quiescent boundary.
    *state.replay_source.lock() = None;
    state.evidence.lock().seal_session();
    let mut engine = state
        .engine
        .lock()
        .map_err(|_| error_response(500, "INTERNAL_ERROR"))?;
    let result = if close {
        engine.close()
    } else {
        engine.revoke()
    };
    match result {
        Ok(()) => {
            if close {
                *state.active_voice_interrupt.lock() = None;
                state.voice_cancel_requested.store(false, Ordering::Release);
                state.voice_busy.store(false, Ordering::Release);
                state.session_end_requested.store(false, Ordering::Release);
            }
            Ok(json_response(200, &serde_json::json!({"ok": true})))
        }
        Err(error) => {
            if matches!(engine.status().session_state.as_str(), "none" | "closed") {
                state.session_end_requested.store(false, Ordering::Release);
            }
            Err(lab_error_response(&error))
        }
    }
}
