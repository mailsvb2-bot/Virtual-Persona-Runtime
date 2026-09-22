use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;

use serde::{Deserialize, Serialize};
use tiny_http::Request;
use vpr_owner_lab::{LabError, LabVoiceResult, LabVoiceSegment};

use super::{
    AppState, HttpResponse, error_response, http_evidence, json_response, read_body,
    reject_if_session_ending,
};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum VoiceStreamEvent {
    Segment { segment: LabVoiceSegment },
    Complete { result: Box<LabVoiceResult> },
    Failed { code: String },
}

#[derive(Default)]
pub(super) struct VoiceStreamState {
    events: VecDeque<VoiceStreamEvent>,
    terminal: bool,
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
        state.voice_busy.store(false, Ordering::Release);
        return response;
    }
    let request_sequence = match http_evidence::request_sequence(request) {
        Ok(sequence) => sequence,
        Err(error) => {
            state.voice_busy.store(false, Ordering::Release);
            return error_response(http_evidence::error_status(error), error.code());
        }
    };
    let audio = match read_body(request, super::MAX_VOICE_BODY_BYTES) {
        Ok(audio) => audio,
        Err(response) => {
            state.voice_busy.store(false, Ordering::Release);
            return response;
        }
    };
    if let Err(error) = state.evidence.lock().begin_voice_request(request_sequence) {
        state.voice_busy.store(false, Ordering::Release);
        return error_response(http_evidence::error_status(error), error.code());
    }
    {
        let mut streams = state.voice_streams.lock();
        if streams.contains_key(&request_sequence) {
            state.voice_busy.store(false, Ordering::Release);
            let _ = state
                .evidence
                .lock()
                .fail_voice_request(request_sequence, "INVALID_STATE_TRANSITION");
            return error_response(409, "INVALID_STATE_TRANSITION");
        }
        streams.insert(request_sequence, VoiceStreamState::default());
    }

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
            let result = engine.voice_turn_streaming(
                audio,
                |handle| {
                    *worker_state.active_voice_interrupt.lock() = Some(handle.clone());
                    if worker_state.voice_cancel_requested.load(Ordering::Acquire) {
                        let _ = handle.interrupt();
                    }
                },
                |segment| {
                    push_voice_stream_event(
                        &worker_state,
                        request_sequence,
                        VoiceStreamEvent::Segment { segment },
                    )
                },
            );
            *worker_state.active_voice_interrupt.lock() = None;
            result
        };
        finish_voice_stream(&worker_state, request_sequence, result);
    });

    json_response(
        202,
        &serde_json::json!({"ok": true, "request_sequence": request_sequence}),
    )
}

pub(super) fn events_response(
    request: &mut Request,
    state: &AppState,
) -> Result<HttpResponse, HttpResponse> {
    let body = super::parse_json::<VoiceEventsBody>(request)?;
    let mut streams = state.voice_streams.lock();
    let stream = streams
        .get_mut(&body.request_sequence)
        .ok_or_else(|| error_response(404, "INVALID_STATE_TRANSITION"))?;
    let events: Vec<_> = stream.events.drain(..).collect();
    let terminal = stream.terminal;
    if terminal {
        streams.remove(&body.request_sequence);
    }
    Ok(json_response(
        200,
        &VoiceEventsResponse { events, terminal },
    ))
}

fn push_voice_stream_event(
    state: &AppState,
    request_sequence: u64,
    event: VoiceStreamEvent,
) -> Result<(), LabError> {
    let mut streams = state.voice_streams.lock();
    let stream = streams
        .get_mut(&request_sequence)
        .ok_or(LabError::InvalidState)?;
    if stream.terminal {
        return Err(LabError::InvalidState);
    }
    stream.events.push_back(event);
    Ok(())
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
    let mut streams = state.voice_streams.lock();
    if let Some(stream) = streams.get_mut(&request_sequence) {
        stream.events.push_back(event);
        stream.terminal = true;
    }
}
