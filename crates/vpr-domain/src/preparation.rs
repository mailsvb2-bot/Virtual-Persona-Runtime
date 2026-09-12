use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::{
    PersonaCaptureState, PersonaId, PersonaProfile, PersonaVersion, PreparationJobId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modality {
    Text,
    Voice,
    Video,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalityReadiness {
    NotReady,
    Preparing,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaReadiness {
    persona_id: PersonaId,
    persona_version: PersonaVersion,
    text: ModalityReadiness,
    voice: ModalityReadiness,
    video: ModalityReadiness,
}

impl PersonaReadiness {
    /// Creates modality readiness for one explicitly reviewed Persona revision.
    ///
    /// Text becomes ready immediately because the reviewed canonical profile is sufficient for
    /// text-only RT0 testing. Voice and video remain independently unavailable until preparation
    /// jobs produce evidence for them.
    ///
    /// # Errors
    /// Returns `ReadinessError::ProfileNotReviewed` until owner review is complete.
    pub fn from_reviewed_profile(profile: &PersonaProfile) -> Result<Self, ReadinessError> {
        if profile.capture_state() != PersonaCaptureState::Reviewed {
            return Err(ReadinessError::ProfileNotReviewed);
        }
        Ok(Self {
            persona_id: profile.identity().id().clone(),
            persona_version: profile.identity().version(),
            text: ModalityReadiness::Ready,
            voice: ModalityReadiness::NotReady,
            video: ModalityReadiness::NotReady,
        })
    }

    #[must_use]
    pub fn persona_id(&self) -> &PersonaId {
        &self.persona_id
    }

    #[must_use]
    pub const fn persona_version(&self) -> PersonaVersion {
        self.persona_version
    }

    #[must_use]
    pub const fn modality(&self, modality: Modality) -> ModalityReadiness {
        match modality {
            Modality::Text => self.text,
            Modality::Voice => self.voice,
            Modality::Video => self.video,
        }
    }

    #[must_use]
    pub fn is_current_for(&self, profile: &PersonaProfile) -> bool {
        self.persona_id == *profile.identity().id()
            && self.persona_version == profile.identity().version()
            && profile.capture_state() == PersonaCaptureState::Reviewed
    }

    #[must_use]
    pub fn is_test_ready_for(&self, profile: &PersonaProfile) -> bool {
        self.is_current_for(profile) && self.text == ModalityReadiness::Ready
    }

    /// Applies preparation evidence without mutating canonical Persona identity.
    ///
    /// # Errors
    /// Returns `ReadinessError` when the job belongs to another Persona or an older/newer revision.
    pub fn apply_job(&mut self, job: &PreparationJob) -> Result<(), ReadinessError> {
        if self.persona_id != job.persona_id {
            return Err(ReadinessError::ForeignPersona);
        }
        if self.persona_version != job.persona_version {
            return Err(ReadinessError::StalePersonaVersion);
        }

        let next = match job.state {
            PreparationJobState::Queued => return Ok(()),
            PreparationJobState::Preparing | PreparationJobState::Validating => {
                ModalityReadiness::Preparing
            }
            PreparationJobState::Ready => ModalityReadiness::Ready,
            PreparationJobState::Failed => ModalityReadiness::Failed,
            PreparationJobState::Cancelled => ModalityReadiness::NotReady,
        };
        self.set_modality(job.modality, next);
        Ok(())
    }

    fn set_modality(&mut self, modality: Modality, readiness: ModalityReadiness) {
        match modality {
            Modality::Text => self.text = readiness,
            Modality::Voice => self.voice = readiness,
            Modality::Video => self.video = readiness,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessError {
    ProfileNotReviewed,
    ForeignPersona,
    StalePersonaVersion,
}

impl Display for ReadinessError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ProfileNotReviewed => "persona must be owner-reviewed before RT0 readiness",
            Self::ForeignPersona => "preparation evidence belongs to another persona",
            Self::StalePersonaVersion => "preparation evidence belongs to another persona version",
        })
    }
}

impl Error for ReadinessError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreparationJobState {
    Queued,
    Preparing,
    Validating,
    Ready,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparationJob {
    id: PreparationJobId,
    persona_id: PersonaId,
    persona_version: PersonaVersion,
    modality: Modality,
    state: PreparationJobState,
}

impl PreparationJob {
    #[must_use]
    pub fn new(id: PreparationJobId, readiness: &PersonaReadiness, modality: Modality) -> Self {
        Self {
            id,
            persona_id: readiness.persona_id.clone(),
            persona_version: readiness.persona_version,
            modality,
            state: PreparationJobState::Queued,
        }
    }

    #[must_use]
    pub fn id(&self) -> &PreparationJobId {
        &self.id
    }

    #[must_use]
    pub fn persona_id(&self) -> &PersonaId {
        &self.persona_id
    }

    #[must_use]
    pub const fn persona_version(&self) -> PersonaVersion {
        self.persona_version
    }

    #[must_use]
    pub const fn modality(&self) -> Modality {
        self.modality
    }

    #[must_use]
    pub const fn state(&self) -> PreparationJobState {
        self.state
    }

    /// Starts provider-neutral preparation work.
    ///
    /// # Errors
    /// Returns `PreparationTransitionError` if the job already left the queue.
    pub fn begin(&mut self) -> Result<(), PreparationTransitionError> {
        self.transition(PreparationJobState::Preparing)
    }

    /// Moves a prepared representation into explicit validation.
    ///
    /// # Errors
    /// Returns `PreparationTransitionError` unless the job is currently preparing.
    pub fn begin_validation(&mut self) -> Result<(), PreparationTransitionError> {
        self.transition(PreparationJobState::Validating)
    }

    /// Records successful validation as readiness evidence.
    ///
    /// # Errors
    /// Returns `PreparationTransitionError` unless validation is in progress.
    pub fn mark_ready(&mut self) -> Result<(), PreparationTransitionError> {
        self.transition(PreparationJobState::Ready)
    }

    /// Records a preparation or validation failure without touching Persona identity.
    ///
    /// # Errors
    /// Returns `PreparationTransitionError` after the job is already terminal or before work starts.
    pub fn fail(&mut self) -> Result<(), PreparationTransitionError> {
        self.transition(PreparationJobState::Failed)
    }

    /// Cancels queued or active preparation.
    ///
    /// # Errors
    /// Returns `PreparationTransitionError` after the job is already terminal.
    pub fn cancel(&mut self) -> Result<(), PreparationTransitionError> {
        self.transition(PreparationJobState::Cancelled)
    }

    fn transition(&mut self, next: PreparationJobState) -> Result<(), PreparationTransitionError> {
        let valid = matches!(
            (self.state, next),
            (
                PreparationJobState::Queued,
                PreparationJobState::Preparing | PreparationJobState::Cancelled
            ) | (
                PreparationJobState::Preparing,
                PreparationJobState::Validating
                    | PreparationJobState::Failed
                    | PreparationJobState::Cancelled
            ) | (
                PreparationJobState::Validating,
                PreparationJobState::Ready
                    | PreparationJobState::Failed
                    | PreparationJobState::Cancelled
            )
        );
        if !valid {
            return Err(PreparationTransitionError {
                from: self.state,
                to: next,
            });
        }
        self.state = next;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparationTransitionError {
    pub from: PreparationJobState,
    pub to: PreparationJobState,
}

impl Display for PreparationTransitionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "invalid preparation transition: {:?} -> {:?}",
            self.from, self.to
        )
    }
}

impl Error for PreparationTransitionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ClaimId, ClaimKind, ConstitutionBoundary, DerivationKind, OwnerClaim, OwnerClaimRecord,
        PersonaIdentity, PersonaMode, SourceKind, VerificationState,
    };

    fn reviewed_profile() -> PersonaProfile {
        let mut profile = PersonaProfile::new(
            PersonaIdentity::new(
                PersonaId::new("persona-1").unwrap(),
                PersonaVersion::new(1).unwrap(),
                PersonaMode::DigitalTwin,
            ),
            ConstitutionBoundary::strict_digital_twin(),
        );
        let id = ClaimId::new("owner-opinion").unwrap();
        profile
            .add_captured_claim(
                OwnerClaimRecord::capture(
                    id.clone(),
                    OwnerClaim {
                        statement: "Прямой ответ владельца".into(),
                        kind: ClaimKind::Opinion,
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
    fn reviewed_persona_is_text_ready_without_voice_or_video() {
        let profile = reviewed_profile();
        let readiness = PersonaReadiness::from_reviewed_profile(&profile).unwrap();
        assert!(readiness.is_test_ready_for(&profile));
        assert_eq!(readiness.modality(Modality::Text), ModalityReadiness::Ready);
        assert_eq!(
            readiness.modality(Modality::Voice),
            ModalityReadiness::NotReady
        );
        assert_eq!(
            readiness.modality(Modality::Video),
            ModalityReadiness::NotReady
        );
    }

    #[test]
    fn voice_and_video_fail_independently_without_damaging_text_readiness() {
        let profile = reviewed_profile();
        let mut readiness = PersonaReadiness::from_reviewed_profile(&profile).unwrap();
        let mut voice = PreparationJob::new(
            PreparationJobId::new("voice-1").unwrap(),
            &readiness,
            Modality::Voice,
        );
        voice.begin().unwrap();
        readiness.apply_job(&voice).unwrap();
        voice.begin_validation().unwrap();
        voice.mark_ready().unwrap();
        readiness.apply_job(&voice).unwrap();

        let mut video = PreparationJob::new(
            PreparationJobId::new("video-1").unwrap(),
            &readiness,
            Modality::Video,
        );
        video.begin().unwrap();
        video.fail().unwrap();
        readiness.apply_job(&video).unwrap();

        assert_eq!(
            readiness.modality(Modality::Voice),
            ModalityReadiness::Ready
        );
        assert_eq!(
            readiness.modality(Modality::Video),
            ModalityReadiness::Failed
        );
        assert!(readiness.is_test_ready_for(&profile));
    }

    #[test]
    fn persona_correction_invalidates_prior_readiness_evidence() {
        let mut profile = reviewed_profile();
        let readiness = PersonaReadiness::from_reviewed_profile(&profile).unwrap();
        let id = ClaimId::new("owner-opinion").unwrap();
        profile
            .correct_claim(&id, "Исправленная позиция", ClaimKind::Opinion)
            .unwrap();
        assert!(!readiness.is_current_for(&profile));
        assert!(!readiness.is_test_ready_for(&profile));
    }

    #[test]
    fn stale_job_cannot_mutate_new_revision_readiness() {
        let mut profile = reviewed_profile();
        let old_readiness = PersonaReadiness::from_reviewed_profile(&profile).unwrap();
        let mut old_job = PreparationJob::new(
            PreparationJobId::new("voice-old").unwrap(),
            &old_readiness,
            Modality::Voice,
        );
        old_job.begin().unwrap();
        old_job.begin_validation().unwrap();
        old_job.mark_ready().unwrap();

        let id = ClaimId::new("owner-opinion").unwrap();
        profile
            .correct_claim(&id, "Исправленная позиция", ClaimKind::Opinion)
            .unwrap();
        let mut new_readiness = PersonaReadiness::from_reviewed_profile(&profile).unwrap();
        assert_eq!(
            new_readiness.apply_job(&old_job),
            Err(ReadinessError::StalePersonaVersion)
        );
        assert_eq!(
            new_readiness.modality(Modality::Voice),
            ModalityReadiness::NotReady
        );
    }

    #[test]
    fn preparation_job_cannot_skip_validation_or_leave_terminal_state() {
        let profile = reviewed_profile();
        let readiness = PersonaReadiness::from_reviewed_profile(&profile).unwrap();
        let mut job = PreparationJob::new(
            PreparationJobId::new("video-guard").unwrap(),
            &readiness,
            Modality::Video,
        );
        assert!(job.mark_ready().is_err());
        job.begin().unwrap();
        job.begin_validation().unwrap();
        job.cancel().unwrap();
        assert!(job.mark_ready().is_err());
    }
}
