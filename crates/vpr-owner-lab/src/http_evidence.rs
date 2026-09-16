use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use tiny_http::Request;
use vpr_domain::Rt0ReasonCode;
use vpr_owner_lab::{
    LabAvSyncEvidenceInput, LabError, LabEvidenceError, LabMediaEvidenceInput,
    LabMediaEvidenceKind, LabSessionEvidenceRecorder, LabSessionEvidenceSnapshot, OwnerLabEngine,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaRecordError {
    Evidence(LabEvidenceError),
    Lab(LabError),
    Internal,
}

impl MediaRecordError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Evidence(error) => error.code(),
            Self::Lab(error) => error.code(),
            Self::Internal => "INTERNAL_ERROR",
        }
    }

    pub const fn status(&self) -> u16 {
        match self {
            Self::Evidence(error) => error_status(*error),
            Self::Lab(
                LabError::EgressDisabled
                | LabError::ConsentRequired
                | LabError::Runtime(
                    Rt0ReasonCode::AuthRevoked
                    | Rt0ReasonCode::AuthExpired
                    | Rt0ReasonCode::AuthScopeDenied,
                ),
            ) => 403,
            Self::Lab(LabError::InvalidInput) => 400,
            Self::Lab(LabError::InvalidState | LabError::Runtime(_)) => 409,
            Self::Lab(LabError::Provider(Rt0ReasonCode::ProviderRateLimited)) => 429,
            Self::Lab(LabError::Provider(Rt0ReasonCode::ProviderTimeout)) => 504,
            Self::Lab(LabError::Provider(_)) => 502,
            Self::Lab(LabError::Internal) | Self::Internal => 500,
        }
    }
}

pub fn record_media(
    engine: &StdMutex<OwnerLabEngine>,
    recorder: &Mutex<LabSessionEvidenceRecorder>,
    input: &LabMediaEvidenceInput,
) -> Result<(), MediaRecordError> {
    if input.kind == LabMediaEvidenceKind::AudioStarted {
        let (canonical_turn_sequence, canonical_output_sequence) = recorder
            .lock()
            .prepare_canonical_playback(input)
            .map_err(MediaRecordError::Evidence)?;
        engine
            .lock()
            .map_err(|_| MediaRecordError::Internal)?
            .acknowledge_voice_playback(canonical_turn_sequence, canonical_output_sequence)
            .map_err(MediaRecordError::Lab)?;
        return recorder
            .lock()
            .record_canonical_playback(input, canonical_turn_sequence, canonical_output_sequence)
            .map_err(MediaRecordError::Evidence);
    }
    recorder
        .lock()
        .record_media(input)
        .map_err(MediaRecordError::Evidence)
}

pub fn record_av_sync(
    recorder: &Mutex<LabSessionEvidenceRecorder>,
    input: &LabAvSyncEvidenceInput,
) -> Result<(), LabEvidenceError> {
    recorder.lock().record_av_sync(input)
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
