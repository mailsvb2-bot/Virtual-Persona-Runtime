use std::collections::BTreeMap;

pub use vpr_evaluation::{
    LabAvSyncEvidence, LabAvSyncEvidenceInput, LabAvSyncReference, LabMediaEvidence,
    LabMediaEvidenceInput, LabMediaEvidenceKind, LabSessionEvidenceSnapshot,
    LabTextAttemptEvidence, LabTextAttemptStatus, LabVoiceAttemptEvidence, LabVoiceAttemptStatus,
    LabVoiceOutputEvidence, ParticipantRole, RT0_AV_SYNC_SAMPLES_PER_REQUEST, RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE,
    RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA,
};

use crate::{LabTextResult, LabVoiceResult};

const MAX_MEDIA_ELAPSED_MILLIS: u64 = 300_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabEvidenceError {
    InvalidInput,
    InvalidState,
    DuplicateEvidence,
}

impl LabEvidenceError {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidInput => "INVALID_INPUT",
            Self::InvalidState => "INVALID_STATE_TRANSITION",
            Self::DuplicateEvidence => "DUPLICATE_EVIDENCE",
        }
    }
}

#[derive(Debug, Default)]
pub struct LabSessionEvidenceRecorder {
    session_sequence: Option<u64>,
    participant_role: Option<ParticipantRole>,
    sealed: bool,
    text_attempts: BTreeMap<u64, LabTextAttemptEvidence>,
    voice_attempts: BTreeMap<u64, LabVoiceAttemptEvidence>,
    media_events: Vec<LabMediaEvidence>,
    av_sync_samples: Vec<LabAvSyncEvidence>,
}

impl LabSessionEvidenceRecorder {
    /// Starts a fresh evidence record for one canonical Owner Lab session.
    ///
    /// # Errors
    /// Returns `InvalidInput` for a zero session sequence.
    pub fn begin_session(
        &mut self,
        session_sequence: u64,
        participant_role: ParticipantRole,
    ) -> Result<(), LabEvidenceError> {
        if session_sequence == 0 {
            return Err(LabEvidenceError::InvalidInput);
        }
        self.session_sequence = Some(session_sequence);
        self.participant_role = Some(participant_role);
        self.sealed = false;
        self.text_attempts.clear();
        self.voice_attempts.clear();
        self.media_events.clear();
        self.av_sync_samples.clear();
        Ok(())
    }

    /// Seals the current session against new voice/media evidence while preserving terminalization
    /// of an already registered in-flight voice request.
    pub fn seal_session(&mut self) {
        if self.session_sequence.is_some() {
            self.sealed = true;
        }
    }

    /// Registers a browser text request before provider work starts.
    ///
    /// # Errors
    /// Fails when there is no current session, the request sequence is zero, or it is duplicated.
    pub fn begin_text_request(&mut self, request_sequence: u64) -> Result<(), LabEvidenceError> {
        if self.session_sequence.is_none() || self.sealed {
            return Err(LabEvidenceError::InvalidState);
        }
        if request_sequence == 0 {
            return Err(LabEvidenceError::InvalidInput);
        }
        if self.text_attempts.contains_key(&request_sequence) {
            return Err(LabEvidenceError::DuplicateEvidence);
        }
        self.text_attempts.insert(
            request_sequence,
            LabTextAttemptEvidence {
                request_sequence,
                canonical_turn_sequence: None,
                canonical_output_sequence: None,
                status: LabTextAttemptStatus::Pending,
                failure_code: None,
                first_meaningful_response_millis: None,
                server_total_millis: None,
                llm_usage: None,
            },
        );
        Ok(())
    }

    /// Completes a registered text request using sanitized server-side evidence only.
    ///
    /// # Errors
    /// Fails when the request is unknown or no longer pending.
    pub fn complete_text_request(
        &mut self,
        request_sequence: u64,
        result: &LabTextResult,
    ) -> Result<(), LabEvidenceError> {
        let attempt = self
            .text_attempts
            .get_mut(&request_sequence)
            .ok_or(LabEvidenceError::InvalidState)?;
        if attempt.status != LabTextAttemptStatus::Pending {
            return Err(LabEvidenceError::DuplicateEvidence);
        }
        if result.evidence_turn_sequence == 0
            || result.evidence_output_sequence == 0
            || result.first_meaningful_response_millis > MAX_MEDIA_ELAPSED_MILLIS
            || result.total_millis > MAX_MEDIA_ELAPSED_MILLIS
            || result.first_meaningful_response_millis > result.total_millis
        {
            return Err(LabEvidenceError::InvalidInput);
        }
        attempt.canonical_turn_sequence = Some(result.evidence_turn_sequence);
        attempt.canonical_output_sequence = Some(result.evidence_output_sequence);
        attempt.status = LabTextAttemptStatus::Completed;
        attempt.first_meaningful_response_millis = Some(result.first_meaningful_response_millis);
        attempt.server_total_millis = Some(result.total_millis);
        attempt.llm_usage = Some(result.llm_usage.clone());
        Ok(())
    }

    /// Marks a registered text request failed without retaining input or generated reply payloads.
    ///
    /// # Errors
    /// Fails when the request is unknown or no longer pending.
    pub fn fail_text_request(
        &mut self,
        request_sequence: u64,
        failure_code: &str,
    ) -> Result<(), LabEvidenceError> {
        let attempt = self
            .text_attempts
            .get_mut(&request_sequence)
            .ok_or(LabEvidenceError::InvalidState)?;
        if attempt.status != LabTextAttemptStatus::Pending || failure_code.trim().is_empty() {
            return Err(LabEvidenceError::DuplicateEvidence);
        }
        attempt.status = LabTextAttemptStatus::Failed;
        attempt.failure_code = Some(failure_code.to_owned());
        Ok(())
    }

    /// Registers a browser voice request before provider work starts.
    ///
    /// # Errors
    /// Fails when there is no current session, the request sequence is zero, or it is duplicated.
    pub fn begin_voice_request(&mut self, request_sequence: u64) -> Result<(), LabEvidenceError> {
        if self.session_sequence.is_none() || self.sealed {
            return Err(LabEvidenceError::InvalidState);
        }
        if request_sequence == 0 {
            return Err(LabEvidenceError::InvalidInput);
        }
        if self.voice_attempts.contains_key(&request_sequence) {
            return Err(LabEvidenceError::DuplicateEvidence);
        }
        self.voice_attempts.insert(
            request_sequence,
            LabVoiceAttemptEvidence {
                request_sequence,
                canonical_turn_sequence: None,
                canonical_outputs: Vec::new(),
                status: LabVoiceAttemptStatus::Pending,
                failure_code: None,
                stt_millis: None,
                llm_millis: None,
                llm_first_meaningful_millis: None,
                avatar_millis: None,
                server_total_millis: None,
                stt_usage: None,
                llm_usage: None,
            },
        );
        Ok(())
    }

    /// Completes a registered request using sanitized server-side voice evidence.
    ///
    /// # Errors
    /// Fails when the request is unknown or no longer pending.
    pub fn complete_voice_request(
        &mut self,
        request_sequence: u64,
        result: &LabVoiceResult,
    ) -> Result<(), LabEvidenceError> {
        let attempt = self
            .voice_attempts
            .get_mut(&request_sequence)
            .ok_or(LabEvidenceError::InvalidState)?;
        if attempt.status != LabVoiceAttemptStatus::Pending {
            return Err(LabEvidenceError::DuplicateEvidence);
        }
        if result.evidence_turn_sequence == 0
            || result.evidence_output_sequences.is_empty()
            || result
                .evidence_output_sequences
                .iter()
                .any(|sequence| *sequence == 0)
            || result.stt_millis > MAX_MEDIA_ELAPSED_MILLIS
            || result.llm_millis > MAX_MEDIA_ELAPSED_MILLIS
            || result.llm_first_meaningful_millis > result.llm_millis
            || result.avatar_millis > MAX_MEDIA_ELAPSED_MILLIS
            || result.total_millis > MAX_MEDIA_ELAPSED_MILLIS
        {
            return Err(LabEvidenceError::InvalidInput);
        }
        let mut outputs = result.evidence_output_sequences.clone();
        outputs.sort_unstable();
        outputs.dedup();
        if outputs.len() != result.evidence_output_sequences.len() {
            return Err(LabEvidenceError::InvalidInput);
        }
        attempt.canonical_turn_sequence = Some(result.evidence_turn_sequence);
        attempt.canonical_outputs = outputs
            .into_iter()
            .map(|canonical_output_sequence| LabVoiceOutputEvidence {
                canonical_output_sequence,
                playback_confirmed: false,
            })
            .collect();
        attempt.status = LabVoiceAttemptStatus::Completed;
        attempt.stt_millis = Some(result.stt_millis);
        attempt.llm_millis = Some(result.llm_millis);
        attempt.llm_first_meaningful_millis = Some(result.llm_first_meaningful_millis);
        attempt.avatar_millis = Some(result.avatar_millis);
        attempt.server_total_millis = Some(result.total_millis);
        attempt.stt_usage = Some(result.stt_usage.clone());
        attempt.llm_usage = Some(result.llm_usage.clone());
        Ok(())
    }

    /// Marks a registered voice request failed without recording provider/user payloads.
    ///
    /// # Errors
    /// Fails when the request is unknown or no longer pending.
    pub fn fail_voice_request(
        &mut self,
        request_sequence: u64,
        failure_code: &str,
    ) -> Result<(), LabEvidenceError> {
        let attempt = self
            .voice_attempts
            .get_mut(&request_sequence)
            .ok_or(LabEvidenceError::InvalidState)?;
        if attempt.status != LabVoiceAttemptStatus::Pending || failure_code.trim().is_empty() {
            return Err(LabEvidenceError::DuplicateEvidence);
        }
        attempt.status = LabVoiceAttemptStatus::Failed;
        attempt.failure_code = Some(failure_code.to_owned());
        Ok(())
    }

    /// Reconciles one provider-confirmed playback completion with one exact canonical segment.
    ///
    /// # Errors
    /// Fails for sealed/unknown requests, cross-turn/output receipts, or duplicate playback proof.
    pub fn record_output_playback(
        &mut self,
        request_sequence: u64,
        canonical_turn_sequence: u64,
        canonical_output_sequence: u64,
    ) -> Result<(), LabEvidenceError> {
        if self.sealed || canonical_turn_sequence == 0 || canonical_output_sequence == 0 {
            return Err(LabEvidenceError::InvalidState);
        }
        let attempt = self
            .voice_attempts
            .get_mut(&request_sequence)
            .ok_or(LabEvidenceError::InvalidState)?;
        if attempt.status != LabVoiceAttemptStatus::Completed
            || attempt.canonical_turn_sequence != Some(canonical_turn_sequence)
        {
            return Err(LabEvidenceError::InvalidState);
        }
        let output = attempt
            .canonical_outputs
            .iter_mut()
            .find(|output| output.canonical_output_sequence == canonical_output_sequence)
            .ok_or(LabEvidenceError::InvalidState)?;
        if output.playback_confirmed {
            return Err(LabEvidenceError::DuplicateEvidence);
        }
        output.playback_confirmed = true;
        Ok(())
    }

    /// Records one browser WebRTC A/V sync sample after canonical playback is proven for the
    /// exact completed request. The reference is explicit in the serialized evidence.
    ///
    /// # Errors
    /// Fails for stale sessions, malformed/duplicate samples, unknown requests, or requests whose
    /// canonical playback has not been confirmed.
    pub fn record_av_sync(
        &mut self,
        input: &LabAvSyncEvidenceInput,
    ) -> Result<(), LabEvidenceError> {
        if self.session_sequence != Some(input.session_sequence) || self.sealed {
            return Err(LabEvidenceError::InvalidState);
        }
        if input.request_sequence == 0
            || !(1..=RT0_AV_SYNC_SAMPLES_PER_REQUEST).contains(&input.sample_sequence)
            || input.absolute_offset_millis > MAX_MEDIA_ELAPSED_MILLIS
        {
            return Err(LabEvidenceError::InvalidInput);
        }
        let attempt = self
            .voice_attempts
            .get(&input.request_sequence)
            .ok_or(LabEvidenceError::InvalidState)?;
        let audio_started = self.media_events.iter().any(|event| {
            event.request_sequence == Some(input.request_sequence)
                && event.kind == LabMediaEvidenceKind::AudioStarted
        });
        if attempt.status != LabVoiceAttemptStatus::Completed || !audio_started {
            return Err(LabEvidenceError::InvalidState);
        }
        if self.av_sync_samples.iter().any(|sample| {
            sample.request_sequence == input.request_sequence
                && sample.sample_sequence == input.sample_sequence
        }) {
            return Err(LabEvidenceError::DuplicateEvidence);
        }
        self.av_sync_samples.push(LabAvSyncEvidence {
            request_sequence: input.request_sequence,
            sample_sequence: input.sample_sequence,
            reference: input.reference,
            absolute_offset_millis: input.absolute_offset_millis,
        });
        Ok(())
    }

    /// Records one browser-observed media-plane event without promoting it to delivery playback.
    ///
    /// `audio_started` measures first meaningful audio only. Canonical output playback is recorded
    /// separately from provider-normalized playback-completion events.
    ///
    /// # Errors
    /// Fails for stale sessions, impossible event/request combinations, incomplete requests, or duplicates.
    pub fn record_media(&mut self, input: &LabMediaEvidenceInput) -> Result<(), LabEvidenceError> {
        self.validate_media(input)?;
        if let Some(request_sequence) = input.request_sequence {
            let attempt = self
                .voice_attempts
                .get(&request_sequence)
                .ok_or(LabEvidenceError::InvalidState)?;
            if attempt.status != LabVoiceAttemptStatus::Completed {
                return Err(LabEvidenceError::InvalidState);
            }
        }
        self.push_media(input);
        Ok(())
    }

    fn validate_media(&self, input: &LabMediaEvidenceInput) -> Result<(), LabEvidenceError> {
        if self.session_sequence != Some(input.session_sequence) || self.sealed {
            return Err(LabEvidenceError::InvalidState);
        }
        if input.elapsed_millis > MAX_MEDIA_ELAPSED_MILLIS {
            return Err(LabEvidenceError::InvalidInput);
        }
        let request_required = matches!(
            input.kind,
            LabMediaEvidenceKind::AudioStarted | LabMediaEvidenceKind::InterruptionStopped
        );
        if request_required != input.request_sequence.is_some() {
            return Err(LabEvidenceError::InvalidInput);
        }
        if let Some(request_sequence) = input.request_sequence {
            if !self.voice_attempts.contains_key(&request_sequence) {
                return Err(LabEvidenceError::InvalidState);
            }
        }
        let duplicate = self.media_events.iter().any(|event| {
            event.kind == input.kind
                && event.request_sequence == input.request_sequence
                && input.kind != LabMediaEvidenceKind::ReconnectRestored
        });
        if duplicate {
            return Err(LabEvidenceError::DuplicateEvidence);
        }
        Ok(())
    }

    fn push_media(&mut self, input: &LabMediaEvidenceInput) {
        self.media_events.push(LabMediaEvidence {
            request_sequence: input.request_sequence,
            kind: input.kind,
            elapsed_millis: input.elapsed_millis,
        });
    }

    /// Returns the current sanitized in-memory evidence snapshot.
    ///
    /// # Errors
    /// Fails when no session evidence has been started yet.
    pub fn snapshot(&self) -> Result<LabSessionEvidenceSnapshot, LabEvidenceError> {
        let session_sequence = self
            .session_sequence
            .ok_or(LabEvidenceError::InvalidState)?;
        let participant_role = self
            .participant_role
            .ok_or(LabEvidenceError::InvalidState)?;
        let completed_attempts: Vec<&LabVoiceAttemptEvidence> = self
            .voice_attempts
            .values()
            .filter(|attempt| attempt.status == LabVoiceAttemptStatus::Completed)
            .collect();
        let canonical_playback_proven = !completed_attempts.is_empty()
            && completed_attempts.iter().all(|attempt| {
                !attempt.canonical_outputs.is_empty()
                    && attempt
                        .canonical_outputs
                        .iter()
                        .all(|output| output.playback_confirmed)
                    && self.media_events.iter().any(|event| {
                        event.request_sequence == Some(attempt.request_sequence)
                            && event.kind == LabMediaEvidenceKind::AudioStarted
                    })
            });
        let av_sync_proven = !completed_attempts.is_empty()
            && completed_attempts.iter().all(|attempt| {
                let audio_started = self.media_events.iter().any(|event| {
                    event.request_sequence == Some(attempt.request_sequence)
                        && event.kind == LabMediaEvidenceKind::AudioStarted
                });
                audio_started
                    && 
(1..=RT0_AV_SYNC_SAMPLES_PER_REQUEST).all(|sample_sequence| {
                        self.av_sync_samples.iter().any(|sample| {
                            sample.request_sequence == attempt.request_sequence
                                && sample.sample_sequence == sample_sequence
                        })
                    })
            });
        Ok(LabSessionEvidenceSnapshot {
            schema_version: RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA.into(),
            scope: RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE.into(),
            session_sequence,
            participant_role,
            canonical_playback_proven,
            av_sync_proven,
            text_attempts: self.text_attempts.values().cloned().collect(),
            voice_attempts: self.voice_attempts.values().cloned().collect(),
            media_events: self.media_events.clone(),
            av_sync_samples: self.av_sync_samples.clone(),
        })
    }
}

#[cfg(test)]
#[path = "evidence_tests.rs"]
mod tests;
