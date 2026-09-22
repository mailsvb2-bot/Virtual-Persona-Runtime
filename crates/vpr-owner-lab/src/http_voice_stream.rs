use std::io::{self, Cursor, Read};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::thread;

use serde::Serialize;
use tiny_http::{Header, Request, Response, StatusCode};
use vpr_owner_lab::{LabError, LabVoicePhrase, LabVoiceResult};

use crate::http_evidence;
use crate::http_json::read_body;
use crate::{AppState, CONTENT_SECURITY_POLICY, MAX_VOICE_BODY_BYTES};

const STREAM_QUEUE_DEPTH: usize = 16;

pub fn respond(mut request: Request, state: Arc<AppState>) {
    if !super::valid_voice_post_headers(&request, &state.csrf_token, state.port) {
        let _ = request.respond(super::error_response(403, "CSRF_DENIED"));
        return;
    }
    if let Err(response) = super::reject_if_session_ending(&state) {
        let _ = request.respond(response);
        return;
    }
    if state
        .voice_busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        let _ = request.respond(super::error_response(409, "INVALID_STATE_TRANSITION"));
        return;
    }
    let reservation = VoiceReservation {
        state: Arc::clone(&state),
    };
    let request_sequence = match http_evidence::request_sequence(&request) {
        Ok(sequence) => sequence,
        Err(error) => {
            let _ = request.respond(super::error_response(
                http_evidence::error_status(error),
                error.code(),
            ));
            return;
        }
    };
    let audio = match read_body(&mut request, MAX_VOICE_BODY_BYTES) {
        Ok(audio) => audio,
        Err(response) => {
            let _ = request.respond(response);
            return;
        }
    };
    if let Err(error) = state.evidence.lock().begin_voice_request(request_sequence) {
        let _ = request.respond(super::error_response(
            http_evidence::error_status(error),
            error.code(),
        ));
        return;
    }

    let (sender, receiver) = mpsc::sync_channel(STREAM_QUEUE_DEPTH);
    thread::spawn(move || {
        let _reservation = reservation;
        run_voice_stream(state, request_sequence, audio, &sender);
    });

    let _ = request.respond(stream_response(ChannelReader::new(receiver)));
}

fn run_voice_stream(
    state: Arc<AppState>,
    request_sequence: u64,
    audio: Vec<u8>,
    sender: &SyncSender<Vec<u8>>,
) {
    if super::reject_if_session_ending(&state).is_err() {
        fail_request(&state, request_sequence, "INVALID_STATE_TRANSITION", sender);
        return;
    }

    let result = {
        let Ok(mut engine) = state.engine.lock() else {
            fail_request(&state, request_sequence, "INTERNAL_ERROR", sender);
            return;
        };
        let result = engine.voice_turn_progressive(
            audio,
            |handle| {
                *state.active_voice_interrupt.lock() = Some(handle.clone());
                if state.voice_cancel_requested.load(Ordering::Acquire) {
                    let _ = handle.interrupt();
                }
            },
            |phrase| send_message(sender, VoiceStreamMessage::Phrase { phrase }),
        );
        *state.active_voice_interrupt.lock() = None;
        result
    };

    match result {
        Ok(value) => {
            if let Err(error) = state
                .evidence
                .lock()
                .complete_voice_request(request_sequence, &value)
            {
                fail_request(&state, request_sequence, error.code(), sender);
                return;
            }
            let _ = send_message(sender, VoiceStreamMessage::Complete { result: &value });
        }
        Err(error) => {
            fail_request(&state, request_sequence, error.code(), sender);
        }
    }
}

fn fail_request(
    state: &AppState,
    request_sequence: u64,
    code: &str,
    sender: &SyncSender<Vec<u8>>,
) {
    let code = match state
        .evidence
        .lock()
        .fail_voice_request(request_sequence, code)
    {
        Ok(()) => code,
        Err(error) => error.code(),
    };
    let _ = send_message(sender, VoiceStreamMessage::Error { code });
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum VoiceStreamMessage<'a> {
    Phrase { phrase: &'a LabVoicePhrase },
    Complete { result: &'a LabVoiceResult },
    Error { code: &'a str },
}

fn send_message(
    sender: &SyncSender<Vec<u8>>,
    message: VoiceStreamMessage<'_>,
) -> Result<(), LabError> {
    let mut bytes = serde_json::to_vec(&message).map_err(|_| LabError::Internal)?;
    bytes.push(b'\n');
    sender.send(bytes).map_err(|_| LabError::Internal)
}

struct VoiceReservation {
    state: Arc<AppState>,
}

impl Drop for VoiceReservation {
    fn drop(&mut self) {
        self.state
            .voice_cancel_requested
            .store(false, Ordering::Release);
        self.state.voice_busy.store(false, Ordering::Release);
    }
}

struct ChannelReader {
    receiver: Receiver<Vec<u8>>,
    current: Cursor<Vec<u8>>,
}

impl ChannelReader {
    fn new(receiver: Receiver<Vec<u8>>) -> Self {
        Self {
            receiver,
            current: Cursor::new(Vec::new()),
        }
    }
}

impl Read for ChannelReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        loop {
            let read = self.current.read(buffer)?;
            if read > 0 {
                return Ok(read);
            }
            match self.receiver.recv() {
                Ok(bytes) => self.current = Cursor::new(bytes),
                Err(_) => return Ok(0),
            }
        }
    }
}

fn stream_response(reader: ChannelReader) -> Response<ChannelReader> {
    let mut response = Response::new(StatusCode(200), Vec::new(), reader, None, None);
    for (name, value) in [
        ("Content-Type", "application/x-ndjson; charset=utf-8"),
        ("Cache-Control", "no-store"),
        ("X-Content-Type-Options", "nosniff"),
        ("Referrer-Policy", "no-referrer"),
        ("Cross-Origin-Opener-Policy", "same-origin"),
        ("Cross-Origin-Resource-Policy", "same-origin"),
        ("X-Frame-Options", "DENY"),
        ("Content-Security-Policy", CONTENT_SECURITY_POLICY),
    ] {
        if let Ok(header) = Header::from_bytes(name, value) {
            response.add_header(header);
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_reader_preserves_ndjson_message_boundaries() {
        let (sender, receiver) = mpsc::sync_channel(2);
        sender.send(b"{\"type\":\"a\"}\n".to_vec()).unwrap();
        sender.send(b"{\"type\":\"b\"}\n".to_vec()).unwrap();
        drop(sender);
        let mut reader = ChannelReader::new(receiver);
        let mut output = String::new();
        reader.read_to_string(&mut output).unwrap();
        assert_eq!(output, "{\"type\":\"a\"}\n{\"type\":\"b\"}\n");
    }
}
