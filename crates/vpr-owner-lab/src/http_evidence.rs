use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use parking_lot::Mutex;
use tiny_http::Request;
use vpr_domain::Rt0ReasonCode;
use vpr_owner_lab::{
    LabAvSyncDiagnosticInput, LabAvSyncEvidenceInput, LabError, LabEvidenceError,
    LabMediaEvidenceInput, LabMediaEvidenceKind, LabSessionEvidenceRecorder,
    LabSessionEvidenceSnapshot, LabVoicePlaybackRegistry, OwnerLabEngine,
};

use super::{
    AppState, HttpResponse, error_response, json_response, parse_empty_json, parse_json, response,
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

pub fn route_post(
    path: &str,
    request: &mut Request,
    state: &AppState,
) -> Option<Result<HttpResponse, HttpResponse>> {
    let result = match path {
        "/api/evidence/media" => parse_json::<LabMediaEvidenceInput>(request).and_then(|body| {
            record_media(&state.engine, &state.voice_playback, &state.evidence, &body)
                .map(|()| json_response(200, &serde_json::json!({"ok": true})))
                .map_err(|error| error_response(error.status(), error.code()))
        }),
        "/api/evidence/av-sync" => parse_json::<LabAvSyncEvidenceInput>(request).and_then(|body| {
            record_av_sync(&state.evidence, &body)
                .map(|()| json_response(200, &serde_json::json!({"ok": true})))
                .map_err(|error| error_response(error_status(error), error.code()))
        }),
        "/api/evidence/av-sync-diagnostic" => parse_json::<LabAvSyncDiagnosticInput>(request)
            .and_then(|body| {
                record_av_sync_diagnostic(&state.evidence, &body)
                    .map(|()| json_response(200, &serde_json::json!({"ok": true})))
                    .map_err(|error| error_response(error_status(error), error.code()))
            }),
        "/api/evidence/session/export" => parse_empty_json(request).and_then(|()| {
            export_terminal_snapshot(&state.engine, &state.evidence, &state.evidence_export)
                .map(|bytes| response(200, bytes, "application/json; charset=utf-8"))
                .map_err(|error| error_response(error.status(), error.code()))
        }),
        _ => return None,
    };
    Some(result)
}

#[derive(Debug, Default)]
pub struct EvidenceExportTracker {
    exported_session_sequence: AtomicU64,
}

impl EvidenceExportTracker {
    fn mark_exported(&self, session_sequence: u64) {
        self.exported_session_sequence
            .store(session_sequence, Ordering::Release);
    }

    fn exported_session_sequence(&self) -> u64 {
        self.exported_session_sequence.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceExportError {
    ExportRequired,
    SessionNotTerminal,
    Evidence(LabEvidenceError),
    Internal,
}

impl EvidenceExportError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::ExportRequired => "EVIDENCE_EXPORT_REQUIRED",
            Self::SessionNotTerminal => "EVIDENCE_SESSION_NOT_TERMINAL",
            Self::Evidence(error) => error.code(),
            Self::Internal => "INTERNAL_ERROR",
        }
    }

    pub const fn status(&self) -> u16 {
        match self {
            Self::ExportRequired | Self::SessionNotTerminal => 409,
            Self::Evidence(error) => error_status(*error),
            Self::Internal => 500,
        }
    }
}

pub fn export_required_for_state(
    session_state: &str,
    recorder: &Mutex<LabSessionEvidenceRecorder>,
    tracker: &EvidenceExportTracker,
) -> bool {
    if !matches!(session_state, "revoked" | "closed") {
        return false;
    }
    recorder
        .lock()
        .snapshot()
        .is_ok_and(|snapshot| snapshot.session_sequence != tracker.exported_session_sequence())
}

pub fn ensure_previous_exported(
    engine: &StdMutex<OwnerLabEngine>,
    recorder: &Mutex<LabSessionEvidenceRecorder>,
    tracker: &EvidenceExportTracker,
) -> Result<(), EvidenceExportError> {
    let session_state = engine
        .lock()
        .map_err(|_| EvidenceExportError::Internal)?
        .status()
        .session_state;
    if export_required_for_state(&session_state, recorder, tracker) {
        Err(EvidenceExportError::ExportRequired)
    } else {
        Ok(())
    }
}

pub fn export_terminal_snapshot(
    engine: &StdMutex<OwnerLabEngine>,
    recorder: &Mutex<LabSessionEvidenceRecorder>,
    tracker: &EvidenceExportTracker,
) -> Result<Vec<u8>, EvidenceExportError> {
    let session_state = engine
        .lock()
        .map_err(|_| EvidenceExportError::Internal)?
        .status()
        .session_state;
    if !matches!(session_state.as_str(), "revoked" | "closed") {
        return Err(EvidenceExportError::SessionNotTerminal);
    }
    let snapshot = recorder
        .lock()
        .snapshot()
        .map_err(EvidenceExportError::Evidence)?;
    let bytes = serde_json::to_vec(&snapshot).map_err(|_| EvidenceExportError::Internal)?;
    tracker.mark_exported(snapshot.session_sequence);
    Ok(bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaRecordError {
    Evidence(LabEvidenceError),
    Lab(LabError),
}

impl MediaRecordError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Evidence(error) => error.code(),
            Self::Lab(error) => error.code(),
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
            Self::Lab(LabError::Provider(Rt0ReasonCode::BudgetExhausted)) => 402,
            Self::Lab(LabError::Provider(Rt0ReasonCode::ProviderRateLimited)) => 429,
            Self::Lab(LabError::Provider(Rt0ReasonCode::ProviderTimeout)) => 504,
            Self::Lab(LabError::Provider(_)) => 502,
            Self::Lab(LabError::PersistenceFailed | LabError::Internal) => 500,
        }
    }
}

pub fn record_media(
    engine: &StdMutex<OwnerLabEngine>,
    playback: &LabVoicePlaybackRegistry,
    recorder: &Mutex<LabSessionEvidenceRecorder>,
    input: &LabMediaEvidenceInput,
) -> Result<(), MediaRecordError> {
    if input.kind == LabMediaEvidenceKind::PlaybackCompleted {
        let (canonical_turn_sequence, canonical_output_sequence) = recorder
            .lock()
            .prepare_canonical_playback(input)
            .map_err(MediaRecordError::Evidence)?;
        playback
            .acknowledge_voice_playback_complete(
                canonical_turn_sequence,
                canonical_output_sequence,
            )
            .map_err(MediaRecordError::Lab)?;
        recorder
            .lock()
            .record_canonical_playback(input, canonical_turn_sequence, canonical_output_sequence)
            .map_err(MediaRecordError::Evidence)?;
        return Ok(());
    }

    recorder
        .lock()
        .record_media(input)
        .map_err(MediaRecordError::Evidence)?;
    if input.kind == LabMediaEvidenceKind::AudioStarted {
        engine
            .lock()
            .map_err(|_| MediaRecordError::Lab(LabError::Internal))?
            .mark_voice_ready_from_media()
            .map_err(MediaRecordError::Lab)?;
    } else if input.kind == LabMediaEvidenceKind::VideoReady {
        engine
            .lock()
            .map_err(|_| MediaRecordError::Lab(LabError::Internal))?
            .mark_video_ready_from_media()
            .map_err(MediaRecordError::Lab)?;
    }
    Ok(())
}

pub fn record_av_sync(
    recorder: &Mutex<LabSessionEvidenceRecorder>,
    input: &LabAvSyncEvidenceInput,
) -> Result<(), LabEvidenceError> {
    recorder.lock().record_av_sync(input)
}

pub fn record_av_sync_diagnostic(
    recorder: &Mutex<LabSessionEvidenceRecorder>,
    input: &LabAvSyncDiagnosticInput,
) -> Result<(), LabEvidenceError> {
    recorder.lock().record_av_sync_diagnostic(input)
}

pub const fn error_status(error: LabEvidenceError) -> u16 {
    match error {
        LabEvidenceError::InvalidInput => 400,
        LabEvidenceError::InvalidState
        | LabEvidenceError::DuplicateEvidence
        | LabEvidenceError::CapacityExceeded => 409,
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

#[cfg(test)]
mod tests {
    use super::*;
    use vpr_owner_lab::ParticipantRole;

    #[test]
    fn terminal_snapshot_export_gate_tracks_each_session_sequence() {
        let recorder = Mutex::new(LabSessionEvidenceRecorder::default());
        let tracker = EvidenceExportTracker::default();
        recorder
            .lock()
            .begin_session(7, ParticipantRole::Owner)
            .unwrap();
        recorder.lock().seal_session();

        assert!(!export_required_for_state("active", &recorder, &tracker));
        assert!(export_required_for_state("closed", &recorder, &tracker));
        tracker.mark_exported(7);
        assert!(!export_required_for_state("closed", &recorder, &tracker));

        recorder
            .lock()
            .begin_session(8, ParticipantRole::Visitor)
            .unwrap();
        recorder.lock().seal_session();
        assert!(export_required_for_state("revoked", &recorder, &tracker));
    }
}
