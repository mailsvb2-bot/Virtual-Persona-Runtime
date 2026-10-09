use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;

use parking_lot::Mutex;
use serde::Deserialize;
use tiny_http::Request;
use vpr_domain::Rt0ReasonCode;
use vpr_owner_lab::{LabError, LabEvidenceError, LabVoiceInput, LabVoiceResult};

use super::{
    AppState, HttpResponse, error_response, http_evidence, json_response, reject_if_session_ending,
};

#[path = "http_voice_event.rs"]
mod voice_event;
use voice_event::VoiceStreamEvent;
#[path = "http_voice_stream.rs"]
mod voice_stream;
#[cfg(test)]
use voice_stream::MAX_PENDING_VOICE_STREAM_EVENTS;
pub(super) use voice_stream::VoiceStreamRegistry;

struct VoiceInputState {
    request_sequence: u64,
    input: LabVoiceInput,
}

#[derive(Default)]
pub(super) struct VoiceInputRegistry {
    active: Mutex<Option<VoiceInputState>>,
}

impl VoiceInputRegistry {
    fn begin(&self, request_sequence: u64, input: LabVoiceInput) -> Result<(), LabVoiceInput> {
        let mut active = self.active.lock();
        if active.is_some() {
            return Err(input);
        }
        *active = Some(VoiceInputState {
            request_sequence,
            input,
        });
        Ok(())
    }

    fn push(&self, request_sequence: u64, pcm: &[u8]) -> Result<(), LabError> {
        let mut active = self.active.lock();
        let state = active.as_mut().ok_or(LabError::InvalidState)?;
        if state.request_sequence != request_sequence {
            return Err(LabError::InvalidState);
        }
        state.input.push_audio(pcm)
    }

    fn take(&self, request_sequence: u64) -> Option<LabVoiceInput> {
        let mut active = self.active.lock();
        if active
            .as_ref()
            .is_some_and(|state| state.request_sequence == request_sequence)
        {
            return active.take().map(|state| state.input);
        }
        None
    }

    fn take_active(&self) -> Option<(u64, LabVoiceInput)> {
        self.active
            .lock()
            .take()
            .map(|state| (state.request_sequence, state.input))
    }
}

#[derive(Deserialize)]
struct VoiceEventsBody {
    request_sequence: u64,
}

pub(super) fn start_input_response(request: &mut Request, state: &Arc<AppState>) -> HttpResponse {
    if let Err(response) = super::parse_empty_json(request) {
        return response;
    }
    if let Err(response) = reject_if_session_ending(state) {
        return response;
    }
    if state
        .voice_busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return error_response(409, "INVALID_STATE_TRANSITION");
    }
    if let Err(response) = reject_if_session_ending(state) {
        release_voice_busy(state);
        return response;
    }
    *state.replay_source.lock() = None;
    let request_sequence = match http_evidence::request_sequence(request) {
        Ok(sequence) => sequence,
        Err(error) => {
            release_voice_busy(state);
            return error_response(http_evidence::error_status(error), error.code());
        }
    };
    if let Err(error) = state.evidence.lock().begin_voice_request(request_sequence) {
        release_voice_busy(state);
        return error_response(http_evidence::error_status(error), error.code());
    }
    let input = {
        let Ok(mut engine) = state.engine.lock() else {
            fail_voice_evidence(state, request_sequence, &LabError::Internal);
            release_voice_busy(state);
            return error_response(500, "INTERNAL_ERROR");
        };
        match engine.begin_voice_input(|handle| {
            *state.active_voice_interrupt.lock() = Some(handle.clone());
            if state.voice_cancel_requested.load(Ordering::Acquire) {
                let _ = handle.interrupt();
            }
        }) {
            Ok(input) => input,
            Err(error) => {
                fail_voice_evidence(state, request_sequence, &error);
                release_voice_busy(state);
                return error_response(lab_error_status(&error), error.code());
            }
        }
    };
    if let Err(input) = state.voice_inputs.begin(request_sequence, input) {
        return fail_voice_upload(state, request_sequence, input, LabError::InvalidState);
    }
    if state.voice_cancel_requested.load(Ordering::Acquire) {
        cancel_active_input(state, LabError::Runtime(Rt0ReasonCode::TurnCancelled));
        return error_response(409, "TURN_CANCELLED");
    }
    json_response(
        201,
        &serde_json::json!({"ok": true, "request_sequence": request_sequence}),
    )
}

pub(super) fn input_chunk_response(request: &mut Request, state: &AppState) -> HttpResponse {
    const MAX_CHUNK_BYTES: u64 = 64 * 1024;

    if let Err(response) = reject_if_session_ending(state) {
        return response;
    }
    let request_sequence = match http_evidence::request_sequence(request) {
        Ok(sequence) => sequence,
        Err(error) => return error_response(http_evidence::error_status(error), error.code()),
    };
    let pcm = match super::read_body(request, MAX_CHUNK_BYTES) {
        Ok(pcm) if !pcm.is_empty() && pcm.len() % 2 == 0 => pcm,
        Ok(_) => return error_response(400, "INVALID_INPUT"),
        Err(response) => return response,
    };
    match state.voice_inputs.push(request_sequence, &pcm) {
        Ok(()) => json_response(200, &serde_json::json!({"ok": true})),
        Err(error) => {
            if let Some(input) = state.voice_inputs.take(request_sequence) {
                fail_voice_upload(state, request_sequence, input, error)
            } else {
                error_response(lab_error_status(&error), error.code())
            }
        }
    }
}

pub(super) fn finish_input_response(request: &mut Request, state: &Arc<AppState>) -> HttpResponse {
    if let Err(response) = super::parse_empty_json(request) {
        return response;
    }
    if let Err(response) = reject_if_session_ending(state) {
        return response;
    }
    let request_sequence = match http_evidence::request_sequence(request) {
        Ok(sequence) => sequence,
        Err(error) => return error_response(http_evidence::error_status(error), error.code()),
    };
    let Some(input) = state.voice_inputs.take(request_sequence) else {
        return error_response(409, "INVALID_STATE_TRANSITION");
    };
    if input.received_bytes() == 0 {
        return fail_voice_upload(state, request_sequence, input, LabError::InvalidInput);
    }
    if !state.voice_streams.begin(request_sequence) {
        return fail_voice_upload(state, request_sequence, input, LabError::InvalidState);
    }
    spawn_voice_worker(state, request_sequence, input);
    json_response(
        202,
        &serde_json::json!({"ok": true, "request_sequence": request_sequence}),
    )
}

pub(super) fn cancel_input_response(request: &mut Request, state: &AppState) -> HttpResponse {
    if let Err(response) = super::parse_empty_json(request) {
        return response;
    }
    let request_sequence = match http_evidence::request_sequence(request) {
        Ok(sequence) => sequence,
        Err(error) => return error_response(http_evidence::error_status(error), error.code()),
    };
    let Some(input) = state.voice_inputs.take(request_sequence) else {
        return error_response(409, "INVALID_STATE_TRANSITION");
    };
    state.voice_cancel_requested.store(true, Ordering::Release);
    if let Some(handle) = state.active_voice_interrupt.lock().clone() {
        let _ = handle.interrupt();
    }
    let error = input.abort(LabError::Runtime(Rt0ReasonCode::TurnCancelled));
    *state.active_voice_interrupt.lock() = None;
    fail_voice_evidence(state, request_sequence, &error);
    release_voice_busy(state);
    json_response(200, &serde_json::json!({"ok": true}))
}

pub(super) fn cancel_active_input(state: &AppState, error: LabError) -> bool {
    let Some((request_sequence, input)) = state.voice_inputs.take_active() else {
        return false;
    };
    let error = input.abort(error);
    *state.active_voice_interrupt.lock() = None;
    fail_voice_evidence(state, request_sequence, &error);
    release_voice_busy(state);
    true
}

pub(super) fn voice_turn_response(request: &mut Request, state: &Arc<AppState>) -> HttpResponse {
    if let Err(response) = reject_if_session_ending(state) {
        return response;
    }
    if state
        .voice_busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return error_response(409, "INVALID_STATE_TRANSITION");
    }
    if let Err(response) = reject_if_session_ending(state) {
        release_voice_busy(state);
        return response;
    }
    *state.replay_source.lock() = None;
    let request_sequence = match http_evidence::request_sequence(request) {
        Ok(sequence) => sequence,
        Err(error) => {
            release_voice_busy(state);
            return error_response(http_evidence::error_status(error), error.code());
        }
    };
    if let Err(error) = state.evidence.lock().begin_voice_request(request_sequence) {
        release_voice_busy(state);
        return error_response(http_evidence::error_status(error), error.code());
    }
    let mut input = {
        let Ok(mut engine) = state.engine.lock() else {
            let error = LabError::Internal;
            let _ = state
                .evidence
                .lock()
                .fail_voice_request(request_sequence, error.code());
            release_voice_busy(state);
            return error_response(lab_error_status(&error), error.code());
        };
        match engine.begin_voice_input(|handle| {
            *state.active_voice_interrupt.lock() = Some(handle.clone());
            if state.voice_cancel_requested.load(Ordering::Acquire) {
                let _ = handle.interrupt();
            }
        }) {
            Ok(input) => input,
            Err(error) => {
                let _ = state
                    .evidence
                    .lock()
                    .fail_voice_request(request_sequence, error.code());
                release_voice_busy(state);
                return error_response(lab_error_status(&error), error.code());
            }
        }
    };

    if let Err(error) = stream_voice_body(request, &mut input) {
        return fail_voice_upload(state, request_sequence, input, error);
    }
    if !state.voice_streams.begin(request_sequence) {
        return fail_voice_upload(state, request_sequence, input, LabError::InvalidState);
    }

    spawn_voice_worker(state, request_sequence, input);

    json_response(
        202,
        &serde_json::json!({"ok": true, "request_sequence": request_sequence}),
    )
}

fn stream_voice_body(request: &mut Request, input: &mut LabVoiceInput) -> Result<(), LabError> {
    const READ_BYTES: usize = 3_200;

    let mut buffer = [0_u8; READ_BYTES];
    let mut pending_byte = None;
    let mut total_bytes = 0_u64;
    loop {
        let read = request
            .as_reader()
            .read(&mut buffer)
            .map_err(|_| LabError::InvalidInput)?;
        if read == 0 {
            break;
        }
        total_bytes = total_bytes
            .checked_add(u64::try_from(read).map_err(|_| LabError::InvalidInput)?)
            .ok_or(LabError::InvalidInput)?;
        if total_bytes > super::MAX_VOICE_BODY_BYTES {
            return Err(LabError::InvalidInput);
        }

        let mut start = 0;
        if let Some(first) = pending_byte.take() {
            input.push_audio(&[first, buffer[0]])?;
            start = 1;
        }
        let even_end = start + ((read - start) / 2) * 2;
        if even_end > start {
            input.push_audio(&buffer[start..even_end])?;
        }
        if even_end < read {
            pending_byte = Some(buffer[read - 1]);
        }
    }
    if total_bytes == 0 || pending_byte.is_some() {
        return Err(LabError::InvalidInput);
    }
    Ok(())
}
fn spawn_voice_worker(state: &Arc<AppState>, request_sequence: u64, input: LabVoiceInput) {
    let worker_state = Arc::clone(state);
    thread::spawn(move || {
        let mut busy = http_evidence::VoiceBusyGuard::new(
            &worker_state.voice_busy,
            &worker_state.voice_cancel_requested,
        );
        let result = match worker_state.engine.lock() {
            Ok(mut engine) => engine.finish_voice_input_streaming(input, |segment| {
                worker_state
                    .evidence
                    .lock()
                    .bind_voice_segment(request_sequence, &segment)
                    .map_err(map_evidence_error)?;
                worker_state
                    .voice_streams
                    .push(request_sequence, VoiceStreamEvent::Segment { segment })
            }),
            Err(_) => Err(LabError::Internal),
        };
        *worker_state.active_voice_interrupt.lock() = None;
        let terminal = prepare_voice_terminal_event(&worker_state, request_sequence, result);
        worker_state
            .voice_streams
            .finish_with_unlock(request_sequence, terminal, || busy.release());
    });
}
const fn map_evidence_error(error: LabEvidenceError) -> LabError {
    match error {
        LabEvidenceError::InvalidInput => LabError::InvalidInput,
        LabEvidenceError::InvalidState
        | LabEvidenceError::DuplicateEvidence
        | LabEvidenceError::CapacityExceeded => LabError::InvalidState,
    }
}

fn fail_voice_evidence(state: &AppState, request_sequence: u64, error: &LabError) {
    let _ = state
        .evidence
        .lock()
        .fail_voice_request(request_sequence, error.code());
}

fn fail_voice_upload(
    state: &AppState,
    request_sequence: u64,
    input: LabVoiceInput,
    error: LabError,
) -> HttpResponse {
    let error = input.abort(error);
    *state.active_voice_interrupt.lock() = None;
    let code = match state
        .evidence
        .lock()
        .fail_voice_request(request_sequence, error.code())
    {
        Ok(()) => error.code(),
        Err(evidence_error) => evidence_error.code(),
    };
    release_voice_busy(state);
    error_response(lab_error_status(&error), code)
}

fn release_voice_busy(state: &AppState) {
    state.voice_cancel_requested.store(false, Ordering::Release);
    state.voice_busy.store(false, Ordering::Release);
}

const fn lab_error_status(error: &LabError) -> u16 {
    match error {
        LabError::EgressDisabled
        | LabError::ConsentRequired
        | LabError::Runtime(
            Rt0ReasonCode::AuthRevoked
            | Rt0ReasonCode::AuthExpired
            | Rt0ReasonCode::AuthScopeDenied,
        ) => 403,
        LabError::InvalidInput | LabError::SpeechNotRecognized => 400,
        LabError::InvalidState | LabError::Runtime(_) => 409,
        LabError::Provider(Rt0ReasonCode::BudgetExhausted) => 402,
        LabError::Provider(Rt0ReasonCode::ProviderRateLimited) => 429,
        LabError::Provider(Rt0ReasonCode::ProviderTimeout) => 504,
        LabError::Provider(_) => 502,
        LabError::PersistenceFailed | LabError::Internal => 500,
    }
}

pub(super) fn events_response(
    request: &mut Request,
    state: &AppState,
) -> Result<HttpResponse, HttpResponse> {
    let body = super::parse_json::<VoiceEventsBody>(request)?;
    let response = state
        .voice_streams
        .wait_events(body.request_sequence)
        .ok_or_else(|| error_response(404, "INVALID_STATE_TRANSITION"))?;
    Ok(json_response(200, &response))
}

fn prepare_voice_terminal_event(
    state: &AppState,
    request_sequence: u64,
    result: Result<LabVoiceResult, LabError>,
) -> VoiceStreamEvent {
    *state.replay_source.lock() = None;
    match result {
        Ok(value) => match state
            .evidence
            .lock()
            .complete_voice_request(request_sequence, &value)
        {
            Ok(()) => {
                super::http_client_control::retain_completed_reply(
                    &state.replay_source,
                    &state.session_end_requested,
                    request_sequence,
                    &value.reply,
                );
                VoiceStreamEvent::Complete {
                    result: Box::new(value),
                }
            }
            Err(error) => VoiceStreamEvent::Failed {
                code: error.code().to_owned(),
                diagnostic: None,
            },
        },
        Err(error) => {
            let code = match state
                .evidence
                .lock()
                .fail_voice_request(request_sequence, error.code())
            {
                Ok(()) => error.code(),
                Err(evidence_error) => evidence_error.code(),
            };
            VoiceStreamEvent::Failed {
                code: code.to_owned(),
                diagnostic: matches!(error, LabError::SpeechNotRecognized)
                    .then(|| "STT_NO_FINAL_TRANSCRIPT".to_owned()),
            }
        }
    }
}

#[cfg(test)]
#[path = "http_voice_tests.rs"]
mod tests;
