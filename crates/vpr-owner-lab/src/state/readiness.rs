use serde::Serialize;
use vpr_domain::{
    Modality, ModalityReadiness, PersonaProfile, PersonaReadiness, PreparationJob,
    PreparationJobId, PreparationJobState,
};

use super::LabError;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LabModalityState {
    NotReady,
    Preparing,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct LabModalityReadiness {
    pub text: LabModalityState,
    pub voice: LabModalityState,
    pub video: LabModalityState,
}

impl Default for LabModalityReadiness {
    fn default() -> Self {
        Self {
            text: LabModalityState::NotReady,
            voice: LabModalityState::NotReady,
            video: LabModalityState::NotReady,
        }
    }
}

#[derive(Default)]
pub(super) struct LabReadinessState {
    readiness: Option<PersonaReadiness>,
    voice_job: Option<PreparationJob>,
    video_job: Option<PreparationJob>,
    job_counter: u64,
}

impl LabReadinessState {
    pub(super) fn reset_for_profile(&mut self, profile: &PersonaProfile) -> Result<(), LabError> {
        self.readiness =
            Some(PersonaReadiness::from_reviewed_profile(profile).map_err(|_| LabError::Internal)?);
        self.voice_job = None;
        self.video_job = None;
        Ok(())
    }

    pub(super) fn snapshot(&self) -> LabModalityReadiness {
        let Some(readiness) = self.readiness.as_ref() else {
            return LabModalityReadiness::default();
        };
        LabModalityReadiness {
            text: map_state(readiness.modality(Modality::Text)),
            voice: map_state(readiness.modality(Modality::Voice)),
            video: map_state(readiness.modality(Modality::Video)),
        }
    }

    pub(super) fn begin_media_preparation(&mut self) -> Result<(), LabError> {
        if self.readiness.is_none() {
            return Ok(());
        }
        self.begin_one(Modality::Voice)?;
        self.begin_one(Modality::Video)?;
        Ok(())
    }

    pub(super) fn begin_media_validation(&mut self) -> Result<(), LabError> {
        if self.readiness.is_none() {
            return Ok(());
        }
        self.begin_validation_one(Modality::Voice)?;
        self.begin_validation_one(Modality::Video)?;
        Ok(())
    }

    pub(super) fn fail_pending(&mut self) {
        self.finish_pending(Modality::Voice);
        self.finish_pending(Modality::Video);
    }

    pub(super) fn cancel_pending(&mut self) {
        self.cancel_one(Modality::Voice);
        self.cancel_one(Modality::Video);
    }

    pub(super) fn mark_ready(&mut self, modality: Modality) -> Result<(), LabError> {
        if !matches!(modality, Modality::Voice | Modality::Video) {
            return Err(LabError::InvalidInput);
        }
        let Some(readiness) = self.readiness.as_ref() else {
            return Ok(());
        };
        if readiness.modality(modality) == ModalityReadiness::Ready {
            return Ok(());
        }

        let mut job = self.take_job(modality).ok_or(LabError::InvalidState)?;
        if job.state() != PreparationJobState::Validating {
            self.put_job(modality, job);
            return Err(LabError::InvalidState);
        }
        job.mark_ready().map_err(|_| LabError::Internal)?;
        self.readiness
            .as_mut()
            .ok_or(LabError::InvalidState)?
            .apply_job(&job)
            .map_err(|_| LabError::Internal)?;
        Ok(())
    }

    fn begin_one(&mut self, modality: Modality) -> Result<(), LabError> {
        let Some(readiness) = self.readiness.as_ref() else {
            return Ok(());
        };
        if matches!(
            readiness.modality(modality),
            ModalityReadiness::Ready | ModalityReadiness::Preparing
        ) {
            return Ok(());
        }

        self.job_counter = self.job_counter.checked_add(1).ok_or(LabError::Internal)?;
        let label = match modality {
            Modality::Voice => "voice",
            Modality::Video => "video",
            Modality::Text => return Err(LabError::InvalidInput),
        };
        let id = PreparationJobId::new(format!("owner-lab-{label}-{}", self.job_counter))
            .map_err(|_| LabError::Internal)?;
        let mut job = self
            .readiness
            .as_mut()
            .ok_or(LabError::InvalidState)?
            .start_preparation(id, modality)
            .map_err(|_| LabError::Internal)?;
        job.begin().map_err(|_| LabError::Internal)?;
        self.readiness
            .as_mut()
            .ok_or(LabError::InvalidState)?
            .apply_job(&job)
            .map_err(|_| LabError::Internal)?;
        self.put_job(modality, job);
        Ok(())
    }

    fn begin_validation_one(&mut self, modality: Modality) -> Result<(), LabError> {
        let Some(mut job) = self.take_job(modality) else {
            return Ok(());
        };
        if job.state() != PreparationJobState::Validating {
            job.begin_validation().map_err(|_| LabError::Internal)?;
            self.readiness
                .as_mut()
                .ok_or(LabError::InvalidState)?
                .apply_job(&job)
                .map_err(|_| LabError::Internal)?;
        }
        self.put_job(modality, job);
        Ok(())
    }

    fn finish_pending(&mut self, modality: Modality) {
        let Some(mut job) = self.take_job(modality) else {
            return;
        };
        if matches!(
            job.state(),
            PreparationJobState::Ready
                | PreparationJobState::Failed
                | PreparationJobState::Cancelled
        ) {
            return;
        }
        if job.fail().is_ok()
            && let Some(readiness) = self.readiness.as_mut()
        {
            let _ = readiness.apply_job(&job);
        }
    }

    fn cancel_one(&mut self, modality: Modality) {
        let Some(mut job) = self.take_job(modality) else {
            return;
        };
        if job.cancel().is_ok()
            && let Some(readiness) = self.readiness.as_mut()
        {
            let _ = readiness.apply_job(&job);
        }
    }

    fn take_job(&mut self, modality: Modality) -> Option<PreparationJob> {
        match modality {
            Modality::Voice => self.voice_job.take(),
            Modality::Video => self.video_job.take(),
            Modality::Text => None,
        }
    }

    fn put_job(&mut self, modality: Modality, job: PreparationJob) {
        match modality {
            Modality::Voice => self.voice_job = Some(job),
            Modality::Video => self.video_job = Some(job),
            Modality::Text => unreachable!("text preparation is rejected before slot access"),
        }
    }
}

const fn map_state(state: ModalityReadiness) -> LabModalityState {
    match state {
        ModalityReadiness::NotReady => LabModalityState::NotReady,
        ModalityReadiness::Preparing => LabModalityState::Preparing,
        ModalityReadiness::Ready => LabModalityState::Ready,
        ModalityReadiness::Failed => LabModalityState::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpr_domain::{
        ClaimId, ClaimKind, ConstitutionBoundary, DerivationKind, OwnerClaim, OwnerClaimRecord,
        PersonaId, PersonaIdentity, PersonaMode, PersonaVersion, SourceKind, VerificationState,
    };

    fn reviewed_profile() -> PersonaProfile {
        let mut profile = PersonaProfile::new(
            PersonaIdentity::new(
                PersonaId::new("readiness-owner").unwrap(),
                PersonaVersion::new(1).unwrap(),
                PersonaMode::DigitalTwin,
            ),
            ConstitutionBoundary::strict_digital_twin(),
        );
        let id = ClaimId::new("owner-fact").unwrap();
        profile
            .add_captured_claim(
                OwnerClaimRecord::capture(
                    id.clone(),
                    OwnerClaim {
                        statement: "Факт владельца".into(),
                        kind: ClaimKind::Factual,
                        source: SourceKind::Owner,
                        verification: VerificationState::Unverified,
                        derivation: DerivationKind::Direct,
                    },
                )
                .unwrap(),
            )
            .unwrap();
        profile.mark_capture_complete().unwrap();
        profile.approve_claim(&id).unwrap();
        profile.approve_initial_review().unwrap();
        profile
    }

    #[test]
    fn media_readiness_requires_real_observation_after_validation_begins() {
        let mut state = LabReadinessState::default();
        state.reset_for_profile(&reviewed_profile()).unwrap();
        assert_eq!(
            state.snapshot(),
            LabModalityReadiness {
                text: LabModalityState::Ready,
                voice: LabModalityState::NotReady,
                video: LabModalityState::NotReady,
            }
        );
        state.begin_media_preparation().unwrap();
        state.begin_media_validation().unwrap();
        assert_eq!(state.snapshot().voice, LabModalityState::Preparing);
        assert_eq!(state.snapshot().video, LabModalityState::Preparing);
        state.mark_ready(Modality::Video).unwrap();
        assert_eq!(state.snapshot().video, LabModalityState::Ready);
        assert_eq!(state.snapshot().voice, LabModalityState::Preparing);
        state.mark_ready(Modality::Voice).unwrap();
        assert_eq!(state.snapshot().voice, LabModalityState::Ready);
    }

    #[test]
    fn failed_attempt_can_retry_without_losing_text_readiness() {
        let mut state = LabReadinessState::default();
        state.reset_for_profile(&reviewed_profile()).unwrap();
        state.begin_media_preparation().unwrap();
        state.begin_media_validation().unwrap();
        state.fail_pending();
        assert_eq!(state.snapshot().text, LabModalityState::Ready);
        assert_eq!(state.snapshot().voice, LabModalityState::Failed);
        assert_eq!(state.snapshot().video, LabModalityState::Failed);
        state.begin_media_preparation().unwrap();
        state.begin_media_validation().unwrap();
        assert_eq!(state.snapshot().voice, LabModalityState::Preparing);
        assert_eq!(state.snapshot().video, LabModalityState::Preparing);
    }
}
