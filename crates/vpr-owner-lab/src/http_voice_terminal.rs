//! Canonical voice completion and replay fencing for the originating RT0 session.
use std::sync::atomic::Ordering;

use super::{AppState, VoiceStreamEvent};
use vpr_owner_lab::{LabError, LabVoiceResult};

pub(super) fn prepare_voice_terminal_event(
    state: &AppState,
    session_sequence: u64,
    request_sequence: u64,
    result: Result<LabVoiceResult, LabError>,
) -> VoiceStreamEvent {
    // Session registration and teardown also hold replay_source while updating
    // the generation. Keep this guard through terminal evidence/replay commit:
    // an old worker must neither clear nor repopulate a new session's reply.
    let Some(mut replay) = super::super::http_client_control::begin_voice_terminal_replay(
        &state.replay_source,
        &state.active_session_sequence,
        &state.session_end_requested,
        session_sequence,
    ) else {
        // Revocation denies replay, but the originating voice attempt must
        // still become terminal before the same session's evidence is sealed.
        // Guard generation and recorder mutation with replay_source: a late
        // worker must never alter a newer session after its state is reused.
        let _replay = state.replay_source.lock();
        if session_sequence != 0
            && state.active_session_sequence.load(Ordering::Acquire) == session_sequence
            && state.session_end_requested.load(Ordering::Acquire)
        {
            let _ = state
                .evidence
                .lock()
                .fail_voice_request(request_sequence, "AUTH_REVOKED");
        }
        return VoiceStreamEvent::Failed {
            code: "INVALID_STATE_TRANSITION".to_owned(),
            diagnostic: None,
        };
    };
    *replay = None;
    match result {
        Ok(value) => match state
            .evidence
            .lock()
            .complete_voice_request(request_sequence, &value)
        {
            Ok(()) => {
                super::super::http_client_control::retain_completed_reply_locked(
                    &mut replay,
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
