use std::collections::BTreeMap;

pub use vpr_evaluation::{
    LabMediaEvidence, LabMediaEvidenceInput, LabMediaEvidenceKind, LabSessionEvidenceSnapshot,
    LabVoiceAttemptEvidence, LabVoiceAttemptStatus, RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE,
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

    /// Records one browser-observed media-plane latency event.
    ///
    /// # Errors
    /// Fails for stale sessions, impossible event/request combinations, unknown requests, or duplicates.
    pub fn record_media(&mut self, input: &LabMediaEvidenceInput) -> Result<(), LabEvidenceError> {
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
        self.media_events.push(LabMediaEvidence {
            request_sequence: input.request_sequence,
            kind: input.kind,
            elapsed_millis: input.elapsed_millis,
        });
        Ok(())
    }

    /// Returns the current sanitized in-memory evidence snapshot.
    ///
    /// # Errors
    /// Fails when no session evidence has been started yet.
    pub fn snapshot(&self) -> Result<LabSessionEvidenceSnapshot, LabEvidenceError> {
        let session_sequence = self
            .session_sequence
            .ok_or(LabEvidenceError::InvalidState)?;
        Ok(LabSessionEvidenceSnapshot {
            schema_version: RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA.into(),
            scope: RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE.into(),
            session_sequence,
            canonical_playback_proven: false,
            av_sync_proven: false,
            voice_attempts: self.voice_attempts.values().cloned().collect(),
            media_events: self.media_events.clone(),
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
        recorder
            .record_media(&LabMediaEvidenceInput {
                session_sequence: 3,
                request_sequence: Some(1),
                kind: LabMediaEvidenceKind::AudioStarted,
                elapsed_millis: 410,
            })
            .unwrap();
        let snapshot = recorder.snapshot().unwrap();
        let json = serde_json::to_string(&snapshot).unwrap();
        assert_eq!(snapshot.voice_attempts[0].canonical_turn_sequence, Some(7));
        assert_eq!(snapshot.scope, RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE);
        assert!(!snapshot.canonical_playback_proven);
        assert!(!snapshot.av_sync_proven);
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
    fn stale_unknown_and_duplicate_media_evidence_fail_closed() {
        let mut recorder = LabSessionEvidenceRecorder::default();
        recorder.begin_session(9).unwrap();
        recorder.begin_voice_request(5).unwrap();
        let event = LabMediaEvidenceInput {
            session_sequence: 9,
            request_sequence: Some(5),
            kind: LabMediaEvidenceKind::AudioStarted,
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
