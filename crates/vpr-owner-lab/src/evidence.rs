use std::collections::BTreeMap;

pub use vpr_evaluation::{
    LabAvSyncEvidence, LabAvSyncEvidenceInput, LabAvSyncReference, LabMediaEvidence,
    LabMediaEvidenceInput, LabMediaEvidenceKind, LabSessionEvidenceSnapshot, LabVoiceAttemptEvidence,
    LabVoiceAttemptStatus, RT0_AV_SYNC_SAMPLES_PER_REQUEST, RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE,
    RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA,
};

use crate::LabVoiceResult;
#[cfg(test)]
use crate::LabVoiceUsage;

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
    sealed: bool,
    voice_attempts: BTreeMap<u64, LabVoiceAttemptEvidence>,
    media_events: Vec<LabMediaEvidence>,
    av_sync_samples: Vec<LabAvSyncEvidence>,
}

impl LabSessionEvidenceRecorder {
    /// Starts a fresh evidence record for one canonical Owner Lab session.
    ///
    /// # Errors
    /// Returns `InvalidInput` for a zero session sequence.
    pub fn begin_session(&mut self, session_sequence: u64) -> Result<(), LabEvidenceError> {
        if session_sequence == 0 {
            return Err(LabEvidenceError::InvalidInput);
        }
        self.session_sequence = Some(session_sequence);
        self.sealed = false;
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
                canonical_output_sequence: None,
                canonical_playback_confirmed: false,
                status: LabVoiceAttemptStatus::Pending,
                failure_code: None,
                stt_millis: None,
                llm_millis: None,
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
        attempt.canonical_turn_sequence = Some(result.evidence_turn_sequence);
        attempt.canonical_output_sequence = Some(result.evidence_output_sequence);
        attempt.canonical_playback_confirmed = false;
        attempt.status = LabVoiceAttemptStatus::Completed;
        attempt.stt_millis = Some(result.stt_millis);
        attempt.llm_millis = Some(result.llm_millis);
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

    /// Returns the canonical turn/output binding for a completed browser voice request.
    ///
    /// # Errors
    /// Fails for sealed sessions, unknown requests, failed/pending attempts, or missing turn data.
    pub fn completed_playback_binding(
        &self,
        request_sequence: u64,
    ) -> Result<(u64, u64), LabEvidenceError> {
        if self.sealed {
            return Err(LabEvidenceError::InvalidState);
        }
        let attempt = self
            .voice_attempts
            .get(&request_sequence)
            .ok_or(LabEvidenceError::InvalidState)?;
        if attempt.status != LabVoiceAttemptStatus::Completed {
            return Err(LabEvidenceError::InvalidState);
        }
        match (
            attempt.canonical_turn_sequence,
            attempt.canonical_output_sequence,
        ) {
            (Some(turn), Some(output)) if turn > 0 && output > 0 => Ok((turn, output)),
            _ => Err(LabEvidenceError::InvalidState),
        }
    }

    /// Validates one browser audio-start observation and returns the exact canonical turn that
    /// must be reconciled before the observation may contribute playback proof.
    ///
    /// # Errors
    /// Fails for stale sessions, malformed/duplicate media, unknown requests, or incomplete turns.
    pub fn prepare_canonical_playback(
        &self,
        input: &LabMediaEvidenceInput,
    ) -> Result<(u64, u64), LabEvidenceError> {
        if input.kind != LabMediaEvidenceKind::AudioStarted {
            return Err(LabEvidenceError::InvalidInput);
        }
        self.validate_media(input)?;
        self.completed_playback_binding(
            input
                .request_sequence
                .ok_or(LabEvidenceError::InvalidInput)?,
        )
    }

    /// Atomically records a browser audio-start observation after canonical runtime playback
    /// reconciliation succeeded for the exact completed turn.
    ///
    /// # Errors
    /// Fails for stale, duplicate, unknown, incomplete, or cross-turn acknowledgements.
    pub fn record_canonical_playback(
        &mut self,
        input: &LabMediaEvidenceInput,
        canonical_turn_sequence: u64,
        canonical_output_sequence: u64,
    ) -> Result<(), LabEvidenceError> {
        if input.kind != LabMediaEvidenceKind::AudioStarted {
            return Err(LabEvidenceError::InvalidInput);
        }
        self.validate_media(input)?;
        let request_sequence = input
            .request_sequence
            .ok_or(LabEvidenceError::InvalidInput)?;
        if self.completed_playback_binding(request_sequence)?
            != (canonical_turn_sequence, canonical_output_sequence)
        {
            return Err(LabEvidenceError::InvalidState);
        }
        let attempt = self
            .voice_attempts
            .get_mut(&request_sequence)
            .ok_or(LabEvidenceError::InvalidState)?;
        attempt.canonical_playback_confirmed = true;
        self.push_media(input);
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
        if attempt.status != LabVoiceAttemptStatus::Completed || !attempt.canonical_playback_confirmed
        {
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

    /// Records one browser-observed media-plane latency event without promoting it to canonical
    /// playback proof. Audio-start evidence reaches `canonical_playback_proven` only through
    /// `record_canonical_playback` after runtime reconciliation.
    ///
    /// # Errors
    /// Fails for stale sessions, impossible event/request combinations, unknown requests, or duplicates.
    pub fn record_media(&mut self, input: &LabMediaEvidenceInput) -> Result<(), LabEvidenceError> {
        if input.kind == LabMediaEvidenceKind::AudioStarted {
            return Err(LabEvidenceError::InvalidState);
        }
        self.validate_media(input)?;
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
        let completed_attempts: Vec<&LabVoiceAttemptEvidence> = self
            .voice_attempts
            .values()
            .filter(|attempt| attempt.status == LabVoiceAttemptStatus::Completed)
            .collect();
        let canonical_playback_proven = !completed_attempts.is_empty()
            && completed_attempts
                .iter()
                .all(|attempt| attempt.canonical_playback_confirmed);
        let av_sync_proven = canonical_playback_proven
            && completed_attempts.iter().all(|attempt| {
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
            canonical_playback_proven,
            av_sync_proven,
            voice_attempts: self.voice_attempts.values().cloned().collect(),
            media_events: self.media_events.clone(),
            av_sync_samples: self.av_sync_samples.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice_result() -> LabVoiceResult {
        LabVoiceResult {
            transcript: "приватный транскрипт".into(),
            reply: "приватный ответ".into(),
            locale: "ru".into(),
            evidence_turn_sequence: 7,
            evidence_output_sequence: 1,
            stt_millis: 100,
            llm_millis: 200,
            avatar_millis: 50,
            total_millis: 350,
            stt_usage: LabVoiceUsage {
                input_units: Some(1000),
                output_units: None,
                estimated_cost_microunits: Some(3),
                provider_charge_microunits: None,
            },
            llm_usage: LabVoiceUsage {
                input_units: Some(12),
                output_units: Some(4),
                estimated_cost_microunits: Some(5),
                provider_charge_microunits: Some(6),
            },
        }
    }

    #[test]
    fn session_reset_and_snapshot_are_payload_redacted() {
        let mut recorder = LabSessionEvidenceRecorder::default();
        recorder.begin_session(3).unwrap();
        recorder.begin_voice_request(1).unwrap();
        recorder.complete_voice_request(1, &voice_result()).unwrap();
        let audio_started = LabMediaEvidenceInput {
            session_sequence: 3,
            request_sequence: Some(1),
            kind: LabMediaEvidenceKind::AudioStarted,
            elapsed_millis: 410,
        };
        assert_eq!(
            recorder.record_media(&audio_started),
            Err(LabEvidenceError::InvalidState)
        );
        assert_eq!(
            recorder.prepare_canonical_playback(&audio_started),
            Ok((7, 1))
        );
        assert_eq!(
            recorder.record_canonical_playback(&audio_started, 7, 2),
            Err(LabEvidenceError::InvalidState)
        );
        recorder
            .record_canonical_playback(&audio_started, 7, 1)
            .unwrap();
        let av_sync = LabAvSyncEvidenceInput {
            session_sequence: 3,
            request_sequence: 1,
            sample_sequence: 1,
            reference: LabAvSyncReference::WebRtcEstimatedPlayoutTimestamp,
            absolute_offset_millis: 60,
        };
        recorder.record_av_sync(&av_sync).unwrap();
        assert_eq!(
            recorder.record_av_sync(&av_sync),
            Err(LabEvidenceError::DuplicateEvidence)
        );
        assert!(!recorder.snapshot().unwrap().av_sync_proven);
        for sample_sequence in 2..=RT0_AV_SYNC_SAMPLES_PER_REQUEST {
            recorder
                .record_av_sync(&LabAvSyncEvidenceInput {
                    sample_sequence,
                    absolute_offset_millis: 60 + u64::from(sample_sequence),
                    ..av_sync.clone()
                })
                .unwrap();
        }
        let snapshot = recorder.snapshot().unwrap();
        let json = serde_json::to_string(&snapshot).unwrap();
        assert_eq!(snapshot.voice_attempts[0].canonical_turn_sequence, Some(7));
        assert_eq!(
            snapshot.voice_attempts[0].canonical_output_sequence,
            Some(1)
        );
        assert!(snapshot.voice_attempts[0].canonical_playback_confirmed);
        assert_eq!(snapshot.scope, RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE);
        assert!(snapshot.canonical_playback_proven);
        assert!(snapshot.av_sync_proven);
        assert_eq!(snapshot.av_sync_samples[0].absolute_offset_millis, 60);
        assert!(!json.contains("приватный транскрипт"));
        assert!(!json.contains("приватный ответ"));
        recorder.seal_session();
        assert_eq!(
            recorder.begin_voice_request(2),
            Err(LabEvidenceError::InvalidState)
        );
        assert_eq!(
            recorder.record_media(&LabMediaEvidenceInput {
                session_sequence: 3,
                request_sequence: None,
                kind: LabMediaEvidenceKind::VideoReady,
                elapsed_millis: 1,
            }),
            Err(LabEvidenceError::InvalidState)
        );
        recorder.begin_session(4).unwrap();
        assert!(recorder.snapshot().unwrap().voice_attempts.is_empty());
    }

    #[test]
    fn every_completed_voice_request_requires_its_own_runtime_playback_confirmation() {
        let mut recorder = LabSessionEvidenceRecorder::default();
        recorder.begin_session(8).unwrap();
        for request in [1, 2] {
            recorder.begin_voice_request(request).unwrap();
            let mut result = voice_result();
            result.evidence_turn_sequence = request + 10;
            result.evidence_output_sequence = request;
            recorder.complete_voice_request(request, &result).unwrap();
        }
        let first = LabMediaEvidenceInput {
            session_sequence: 8,
            request_sequence: Some(1),
            kind: LabMediaEvidenceKind::AudioStarted,
            elapsed_millis: 100,
        };
        recorder.record_canonical_playback(&first, 11, 1).unwrap();
        assert!(!recorder.snapshot().unwrap().canonical_playback_proven);

        let second = LabMediaEvidenceInput {
            session_sequence: 8,
            request_sequence: Some(2),
            kind: LabMediaEvidenceKind::AudioStarted,
            elapsed_millis: 120,
        };
        recorder.record_canonical_playback(&second, 12, 2).unwrap();
        assert!(recorder.snapshot().unwrap().canonical_playback_proven);
        assert_eq!(
            recorder.record_canonical_playback(&second, 12, 2),
            Err(LabEvidenceError::DuplicateEvidence)
        );
    }

    #[test]
    fn av_sync_requires_completed_canonical_playback_for_the_same_request() {
        let mut recorder = LabSessionEvidenceRecorder::default();
        recorder.begin_session(12).unwrap();
        recorder.begin_voice_request(1).unwrap();
        let sample = LabAvSyncEvidenceInput {
            session_sequence: 12,
            request_sequence: 1,
            sample_sequence: 1,
            reference: LabAvSyncReference::WebRtcEstimatedPlayoutTimestamp,
            absolute_offset_millis: 40,
        };
        assert_eq!(
            recorder.record_av_sync(&sample),
            Err(LabEvidenceError::InvalidState)
        );
        recorder.complete_voice_request(1, &voice_result()).unwrap();
        assert_eq!(
            recorder.record_av_sync(&sample),
            Err(LabEvidenceError::InvalidState)
        );
        let audio_started = LabMediaEvidenceInput {
            session_sequence: 12,
            request_sequence: Some(1),
            kind: LabMediaEvidenceKind::AudioStarted,
            elapsed_millis: 100,
        };
        recorder
            .record_canonical_playback(&audio_started, 7, 1)
            .unwrap();
        recorder.record_av_sync(&sample).unwrap();
        assert!(!recorder.snapshot().unwrap().av_sync_proven);
        for sample_sequence in 2..=RT0_AV_SYNC_SAMPLES_PER_REQUEST {
            recorder
                .record_av_sync(&LabAvSyncEvidenceInput {
                    sample_sequence,
                    ..sample.clone()
                })
                .unwrap();
        }
        assert!(recorder.snapshot().unwrap().av_sync_proven);
        assert_eq!(
            recorder.record_av_sync(&LabAvSyncEvidenceInput {
                sample_sequence: RT0_AV_SYNC_SAMPLES_PER_REQUEST + 1,
                ..sample
            }),
            Err(LabEvidenceError::InvalidInput)
        );
    }

    #[test]
    fn stale_unknown_and_duplicate_media_evidence_fail_closed() {
        let mut recorder = LabSessionEvidenceRecorder::default();
        recorder.begin_session(9).unwrap();
        recorder.begin_voice_request(5).unwrap();
        let event = LabMediaEvidenceInput {
            session_sequence: 9,
            request_sequence: None,
            kind: LabMediaEvidenceKind::VideoReady,
            elapsed_millis: 250,
        };
        recorder.record_media(&event).unwrap();
        assert_eq!(
            recorder.record_media(&event),
            Err(LabEvidenceError::DuplicateEvidence)
        );
        assert_eq!(
            recorder.record_media(&LabMediaEvidenceInput {
                session_sequence: 8,
                request_sequence: None,
                kind: LabMediaEvidenceKind::VideoReady,
                elapsed_millis: 10,
            }),
            Err(LabEvidenceError::InvalidState)
        );
        assert_eq!(
            recorder.record_media(&LabMediaEvidenceInput {
                session_sequence: 9,
                request_sequence: Some(999),
                kind: LabMediaEvidenceKind::InterruptionStopped,
                elapsed_millis: 10,
            }),
            Err(LabEvidenceError::InvalidState)
        );
    }
}
