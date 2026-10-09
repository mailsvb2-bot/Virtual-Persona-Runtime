use std::sync::{Arc, atomic::Ordering};
use std::thread;
use std::time::{Duration, Instant};

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
    state: &Arc<AppState>,
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

/// Fences the exact session generation before audio/provider work. This
/// invalidates the *shared Rust authorization epoch* without waiting for the
/// engine mutex held by a slow STT/LLM worker. Diagnostics may still be flushed
/// before the final close, but no new canonical provider permits can be issued.
fn fence_authority(
    state: &AppState,
    expected_session_sequence: Option<u64>,
) -> Result<(), HttpResponse> {
    // Start holds replay_source through the new-session registration. This
    // ordering ensures an old tab cannot fence a newer generation.
    let revoker = {
        let mut replay = state.replay_source.lock();
        let registered = state.session_revocation.lock();
        let current = state.active_session_sequence.load(Ordering::Acquire);
        if current == 0 || expected_session_sequence.is_some_and(|id| id == 0 || id != current) {
            return Err(error_response(409, "INVALID_STATE_TRANSITION"));
        }
        let Some(revoker) = registered.clone() else {
            return Err(error_response(409, "INVALID_STATE_TRANSITION"));
        };
        state.session_end_requested.store(true, Ordering::Release);
        *replay = None;
        revoker
    };
    // Canonical revoke is FIRST. Do not take engine.lock() on this path:
    // the in-flight voice worker may hold it beyond the teardown deadline.
    revoker
        .revoke_authority()
        .map_err(|code| lab_error_response(&LabError::Runtime(code)))?;
    request_voice_cancel(state);
    // When the engine is idle, also complete the normal remote-provider cleanup
    // immediately. This preserves the existing /fence user-visible "revoked"
    // state while keeping the busy-worker path strictly NON-BLOCKING: authority
    // has already been withdrawn even if the provider is still unwinding.
    match state.engine.try_lock() {
        Ok(mut engine) => engine
            .revoke()
            .map_err(|error| lab_error_response(&error))?,
        Err(std::sync::TryLockError::WouldBlock) => {}
        Err(std::sync::TryLockError::Poisoned(_)) => {
            return Err(error_response(500, "INTERNAL_ERROR"));
        }
    }
    Ok(())
}

/// First phase of user Close: deny new turns and revoke provider authority now.
/// Unlike Close, this intentionally leaves the bounded evidence recorder open.
pub(super) fn fence_session_response(
    request: &mut tiny_http::Request,
    state: &AppState,
) -> Result<HttpResponse, HttpResponse> {
    let body: SessionEndBody = super::parse_json(request)?;
    // Unlike legacy revoke/close, the new pre-flush fence is always strictly
    // generation-bound. An unscoped stale tab must not fence another session.
    let sequence = body
        .expected_session_sequence
        .filter(|sequence| *sequence > 0)
        .ok_or_else(|| error_response(400, "INVALID_INPUT"))?;
    fence_authority(state, Some(sequence))?;
    Ok(json_response(200, &serde_json::json!({"ok": true})))
}

// The HTTP request must never wait indefinitely for a vendor STT/LLM socket.
// A timed-out worker is already fenced from new canonical egress. Carry its
// local provider teardown to completion once the *same generation* quiesces.
// The bounded worker cannot close a later session; no fake "closed" status is
// published while the old provider resource has not actually been released.
fn complete_deferred_session(
    state: &AppState,
    close: bool,
    expected_generation: u64,
) -> Result<(), HttpResponse> {
    if expected_generation == 0
        || state.active_session_sequence.load(Ordering::Acquire) != expected_generation
        || !state.session_end_requested.load(Ordering::Acquire)
    {
        return Err(error_response(409, "INVALID_STATE_TRANSITION"));
    }
    let mut engine = state
        .engine
        .lock()
        .map_err(|_| error_response(500, "INTERNAL_ERROR"))?;
    if state.active_session_sequence.load(Ordering::Acquire) != expected_generation {
        return Err(error_response(409, "INVALID_STATE_TRANSITION"));
    }
    if close {
        engine.close()
    } else {
        engine.revoke()
    }
    .map_err(|error| lab_error_response(&error))?;

    *state.replay_source.lock() = None;
    state.evidence.lock().seal_session();
    if close {
        *state.active_voice_interrupt.lock() = None;
        state.voice_cancel_requested.store(false, Ordering::Release);
        state.voice_busy.store(false, Ordering::Release);
        *state.session_revocation.lock() = None;
        state.active_session_sequence.store(0, Ordering::Release);
        state.session_end_requested.store(false, Ordering::Release);
    }
    Ok(())
}

fn finish_after_quiescence(state: &AppState, close: bool, expected_generation: u64) {
    // A single request can time out after the canonical authority has already
    // been revoked. Finish only when the voice worker has released its output
    // stream. Do not evict a live stream or claim provider cleanup succeeded.
    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        if state.active_session_sequence.load(Ordering::Acquire) != expected_generation
            || !state.session_end_requested.load(Ordering::Acquire)
        {
            return;
        }
        if state.voice_streams.wait_until_quiescent() {
            let _ = complete_deferred_session(state, close, expected_generation);
            return;
        }
    }
    // Remain fenced and visibly unfinished. A later explicit Close can retry;
    // automatic success would be false if a provider refuses to quiesce.
}

fn end_session(
    state: &Arc<AppState>,
    close: bool,
    expected_session_sequence: Option<u64>,
) -> Result<HttpResponse, HttpResponse> {
    fence_authority(state, expected_session_sequence)?;
    let expected_generation = state.active_session_sequence.load(Ordering::Acquire);
    if !state.voice_streams.wait_until_quiescent() {
        let worker_state = Arc::clone(state);
        thread::spawn(move || {
            finish_after_quiescence(&worker_state, close, expected_generation);
        });
        return Err(error_response(504, "PROVIDER_TIMEOUT"));
    }
    complete_deferred_session(state, close, expected_generation)?;
    Ok(json_response(200, &serde_json::json!({"ok": true})))
}
