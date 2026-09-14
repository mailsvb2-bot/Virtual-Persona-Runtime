use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use tiny_http::Request;
use vpr_owner_lab::{
    LabEvidenceError, LabMediaEvidenceInput, LabSessionEvidenceRecorder, LabSessionEvidenceSnapshot,
};

pub fn request_sequence(request: &Request) -> Result<u64, LabEvidenceError> {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv("X-VPR-Evidence-Request"))
        .map(|header| header.value.as_str())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .ok_or(LabEvidenceError::InvalidInput)
}

pub fn snapshot(
    recorder: &Mutex<LabSessionEvidenceRecorder>,
) -> Result<LabSessionEvidenceSnapshot, LabEvidenceError> {
    recorder.lock().snapshot()
}

pub fn record_media(
    recorder: &Mutex<LabSessionEvidenceRecorder>,
    input: &LabMediaEvidenceInput,
) -> Result<(), LabEvidenceError> {
    recorder.lock().record_media(input)
}

pub const fn error_status(error: LabEvidenceError) -> u16 {
    match error {
        LabEvidenceError::InvalidInput => 400,
        LabEvidenceError::InvalidState | LabEvidenceError::DuplicateEvidence => 409,
    }
}

pub struct VoiceBusyGuard<'a> {
    busy: &'a AtomicBool,
    cancel_requested: &'a AtomicBool,
}

impl<'a> VoiceBusyGuard<'a> {
    pub const fn new(busy: &'a AtomicBool, cancel_requested: &'a AtomicBool) -> Self {
        Self {
            busy,
            cancel_requested,
        }
    }
}

impl Drop for VoiceBusyGuard<'_> {
    fn drop(&mut self) {
        self.cancel_requested.store(false, Ordering::Release);
        self.busy.store(false, Ordering::Release);
    }
}
