use super::{
    LabAvSyncDiagnostic, LabAvSyncDiagnosticInput, LabEvidenceError, LabSessionEvidenceRecorder,
    LabVoiceAttemptStatus, RT0_AV_SYNC_SAMPLES_PER_REQUEST, RT0_OWNER_LAB_MAX_SESSION_ATTEMPTS,
};

impl LabSessionEvidenceRecorder {
    /// Records a sanitized final reason when bounded browser A/V-sync sampling could not
    /// collect all required samples after observed audio start, including an active streamed voice request.
    ///
    /// This is diagnostic evidence, not confirmation that the response finished playing.
    /// Requiring canonical playback here would discard the precise failure diagnostics from
    /// D-ID sessions where audio starts but no authoritative playback receipt is received.
    ///
    /// # Errors
    /// Fails for stale sessions, malformed/duplicate diagnostics, unknown/failed requests,
    /// requests without observed audio start, or requests with complete A/V-sync samples.
    pub fn record_av_sync_diagnostic(
        &mut self,
        input: &LabAvSyncDiagnosticInput,
    ) -> Result<(), LabEvidenceError> {
        if self.session_sequence != Some(input.session_sequence) || self.sealed {
            return Err(LabEvidenceError::InvalidState);
        }
        if input.request_sequence == 0
            || input.attempts == 0
            || input.audio_issue.is_none() && input.video_issue.is_none()
        {
            return Err(LabEvidenceError::InvalidInput);
        }
        let attempt = self
            .voice_attempts
            .get(&input.request_sequence)
            .ok_or(LabEvidenceError::InvalidState)?;
        let audio_started = self.media_events.iter().any(|event| {
            event.request_sequence == Some(input.request_sequence)
                && event.kind == super::LabMediaEvidenceKind::AudioStarted
        });
        // RTP observation may precede backend completion during streamed speech.
        // Failed attempts and sessions without observed audio remain ineligible;
        // only completed, canonically acknowledged turns can promote RT0 proof.
        if attempt.status == LabVoiceAttemptStatus::Failed || !audio_started {
            return Err(LabEvidenceError::InvalidState);
        }
        let complete_samples = (1..=RT0_AV_SYNC_SAMPLES_PER_REQUEST).all(|sample_sequence| {
            self.av_sync_samples.iter().any(|sample| {
                sample.request_sequence == input.request_sequence
                    && sample.sample_sequence == sample_sequence
            })
        });
        if complete_samples {
            return Err(LabEvidenceError::InvalidState);
        }
        if self
            .av_sync_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.request_sequence == input.request_sequence)
        {
            return Err(LabEvidenceError::DuplicateEvidence);
        }
        if self.av_sync_diagnostics.len() >= RT0_OWNER_LAB_MAX_SESSION_ATTEMPTS {
            return Err(LabEvidenceError::CapacityExceeded);
        }
        self.av_sync_diagnostics.push(LabAvSyncDiagnostic {
            request_sequence: input.request_sequence,
            attempts: input.attempts,
            audio_issue: input.audio_issue,
            video_issue: input.video_issue,
        });
        Ok(())
    }
}
