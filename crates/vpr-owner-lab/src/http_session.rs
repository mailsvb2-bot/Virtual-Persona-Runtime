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

/// Registers the just-created session only if no concurrent revoke already won.
/// The registration and end-session fence use the same small mutex, never the
/// worker-owned engine mutex. A start racing revoke fails closed.
pub(super) fn register_started_session(
    state: &AppState,
    engine: &mut vpr_owner_lab::OwnerLabEngine,
) -> Result<(), LabError> {
    let mut registered = state.session_revocation.lock();
    if state.session_end_requested.load(Ordering::Acquire) {
        // The start may have been executing against the provider when the
        // user revoked. Do not return an active transport/token to that caller.
        drop(registered);
        let _ = engine.revoke();
        state.session_end_requested.store(false, Ordering::Release);
        return Err(LabError::InvalidState);
    }
    *registered = Some(
        engine
            .session_revocation_handle()
            .ok_or(LabError::InvalidState)?,
    );
    Ok(())
}

pub(super) fn end_session(state: &AppState, close: bool) -> Result<HttpResponse, HttpResponse> {
    // Linearize start registration against revoke without acquiring the engine
    // mutex: a blocked STT/LLM worker may hold that lock for much longer than
    // the bounded HTTP teardown deadline.
    let revoker = {
        let registered = state.session_revocation.lock();
        state.session_end_requested.store(true, Ordering::Release);
        registered.clone()
    };
    let Some(revoker) = revoker else {
        // No active session has registered any canonical authority.
        // If a concurrent start is opening the provider, its engine mutex is
        // busy; keep the fence set until registration rejects that new session.
        // Only clear the fence when no start/worker holds the engine.
        if state.engine.try_lock().is_ok() {
            state.session_end_requested.store(false, Ordering::Release);
        }
        return Err(error_response(409, "INVALID_STATE_TRANSITION"));
    };
    // This is the actual shared runtime authorization owner, not an HTTP-only
    // flag. It invalidates in-flight provider permits BEFORE any wait or
    // provider-specific cleanup. The engine mutex is deliberately untouched.
    revoker
        .revoke_authority()
        .map_err(|error| lab_error_response(&LabError::Runtime(error)))?;
    request_voice_cancel(state);
    if !state.voice_streams.wait_until_quiescent() {
        return Err(error_response(504, "PROVIDER_TIMEOUT"));
    }
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
                *state.session_revocation.lock() = None;
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
