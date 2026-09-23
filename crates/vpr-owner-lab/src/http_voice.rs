use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use parking_lot::{Condvar, Mutex};
use serde::{Deserialize, Serialize};
use tiny_http::Request;
use vpr_domain::Rt0ReasonCode;
use vpr_owner_lab::{LabError, LabVoiceInput, LabVoiceResult, LabVoiceSegment};

use super::{
    AppState, HttpResponse, error_response, http_evidence, json_response, reject_if_session_ending,
};

const EVENT_WAIT_TIMEOUT: Duration = Duration::from_secs(25);
const TERMINATION_WAIT_TIMEOUT: Duration = Duration::from_millis(1_000);

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum VoiceStreamEvent {
    Segment { segment: LabVoiceSegment },
    Complete { result: Box<LabVoiceResult> },
    Failed { code: String },
}

#[derive(Default)]
struct VoiceStreamState {
    events: VecDeque<VoiceStreamEvent>,
    terminal: bool,
}

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

#[derive(Default)]
pub(super) struct VoiceStreamRegistry {
    streams: Mutex<BTreeMap<u64, VoiceStreamState>>,
    changed: Condvar,
}

impl VoiceStreamRegistry {
    pub(super) fn clear(&self) {
        self.streams.lock().clear();
        self.changed.notify_all();
    }

    pub(super) fn wait_until_quiescent(&self) -> bool {
        let started = std::time::Instant::now();
        let mut streams = self.streams.lock();
        loop {
            if streams.values().all(|stream| stream.terminal) {
                return true;
            }
            let Some(remaining) = TERMINATION_WAIT_TIMEOUT.checked_sub(started.elapsed()) else {
                return false;
            };
            if remaining.is_zero() {
                return false;
            }
            if self.changed.wait_for(&mut streams, remaining).timed_out()
                && streams.values().any(|stream| !stream.terminal)
            {
                return false;
            }
        }
    }

    fn begin(&self, request_sequence: u64) -> bool {
        let mut streams = self.streams.lock();
        if streams.contains_key(&request_sequence) {
            return false;
        }
        streams.insert(request_sequence, VoiceStreamState::default());
        true
    }

    fn push(&self, request_sequence: u64, event: VoiceStreamEvent) -> Result<(), LabError> {
        let mut streams = self.streams.lock();
        let stream = streams
            .get_mut(&request_sequence)
            .ok_or(LabError::InvalidState)?;
        if stream.terminal {
            return Err(LabError::InvalidState);
        }
        stream.events.push_back(event);
        drop(streams);
        self.changed.notify_all();
        Ok(())
    }

    fn finish(&self, request_sequence: u64, event: VoiceStreamEvent) {
        let mut streams = self.streams.lock();
        if let Some(stream) = streams.get_mut(&request_sequence) {
            stream.events.push_back(event);
            stream.terminal = true;
        }
        drop(streams);
        self.changed.notify_all();
    }

    fn wait_events(&self, request_sequence: u64) -> Option<VoiceEventsResponse> {
        let mut streams = self.streams.lock();
        {
            let stream = streams.get(&request_sequence)?;
            if stream.events.is_empty() && !stream.terminal {
                self.changed.wait_for(&mut streams, EVENT_WAIT_TIMEOUT);
            }
        }
        let (events, terminal) = {
            let stream = streams.get_mut(&request_sequence)?;
            (stream.events.drain(..).collect::<Vec<_>>(), stream.terminal)
        };
        if terminal {
            streams.remove(&request_sequence);
        }
        Some(VoiceEventsResponse { events, terminal })
    }
}

#[derive(Deserialize)]
struct VoiceEventsBody {
    request_sequence: u64,
}

#[derive(Serialize)]
struct VoiceEventsResponse {
    events: Vec<VoiceStreamEvent>,
    terminal: bool,
}

pub(super) fn start_input_response(
    request: &mut Request,
    state: &Arc<AppState>,
) -> HttpResponse {
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

pub(super) fn finish_input_response(
    request: &mut Request,
    state: &Arc<AppState>,
) -> HttpResponse {
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
    state.voice_cancel_requested.store(true, Ordering::Release);
    if let Some(handle) = state.active_voice_interrupt.lock().clone() {
        let _ = handle.interrupt();
    }
    let Some(input) = state.voice_inputs.take(request_sequence) else {
        return error_response(409, "INVALID_STATE_TRANSITION");
    };
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
        let _busy = http_evidence::VoiceBusyGuard::new(
            &worker_state.voice_busy,
            &worker_state.voice_cancel_requested,
        );
        let result = {
            let Ok(mut engine) = worker_state.engine.lock() else {
                finish_voice_stream(&worker_state, request_sequence, Err(LabError::Internal));
                return;
            };
            let result = engine.finish_voice_input_streaming(input, |segment| {
                worker_state
                    .evidence
                    .lock()
                    .bind_voice_segment(request_sequence, &segment)
                    .map_err(|_| LabError::Internal)?;
                worker_state
                    .voice_streams
                    .push(request_sequence, VoiceStreamEvent::Segment { segment })
            });
            *worker_state.active_voice_interrupt.lock() = None;
            result
        };
        finish_voice_stream(&worker_state, request_sequence, result);
    });
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
        LabError::InvalidInput => 400,
        LabError::InvalidState | LabError::Runtime(_) => 409,
        LabError::Provider(Rt0ReasonCode::ProviderRateLimited) => 429,
        LabError::Provider(Rt0ReasonCode::ProviderTimeout) => 504,
        LabError::Provider(_) => 502,
        LabError::Internal => 500,
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

fn finish_voice_stream(
    state: &AppState,
    request_sequence: u64,
    result: Result<LabVoiceResult, LabError>,
) {
    let event = match result {
        Ok(value) => match state
            .evidence
            .lock()
            .complete_voice_request(request_sequence, &value)
        {
            Ok(()) => VoiceStreamEvent::Complete {
                result: Box::new(value),
            },
            Err(error) => VoiceStreamEvent::Failed {
                code: error.code().to_owned(),
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
            }
        }
    };
    state.voice_streams.finish(request_sequence, event);
}
