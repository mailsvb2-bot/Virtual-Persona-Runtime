use std::error::Error;
use std::fmt::{Display, Formatter};

use vpr_domain::{
    ClaimId, ClaimKind, DerivationKind, OwnerClaim, OwnerClaimRecord, PersonaCaptureState,
    PersonaProfile, ProfileError, SourceKind, VerificationState,
};

use crate::{InterviewPlan, InterviewQuestion};

#[derive(Debug)]
pub struct GuidedOwnerInterview {
    profile: PersonaProfile,
    plan: InterviewPlan,
    next_question: usize,
}

impl GuidedOwnerInterview {
    /// Starts a guided RT0 interview over the one canonical Persona profile.
    ///
    /// # Errors
    /// Returns `CaptureError` when the Persona mode is unsupported or the profile is not a clean draft.
    pub fn new(profile: PersonaProfile, plan: InterviewPlan) -> Result<Self, CaptureError> {
        if !profile.identity().is_rt0_supported() {
            return Err(CaptureError::UnsupportedPersonaMode);
        }
        if profile.capture_state() != PersonaCaptureState::Draft || !profile.claims().is_empty() {
            return Err(CaptureError::ProfileNotCleanDraft);
        }
        Ok(Self {
            profile,
            plan,
            next_question: 0,
        })
    }

    #[must_use]
    pub fn profile(&self) -> &PersonaProfile {
        &self.profile
    }

    #[must_use]
    pub fn current_question(&self) -> Option<&InterviewQuestion> {
        self.plan.questions().get(self.next_question)
    }

    #[must_use]
    pub const fn answered_count(&self) -> usize {
        self.next_question
    }

    #[must_use]
    pub fn question_count(&self) -> usize {
        self.plan.question_count()
    }

    /// Captures one answer as direct owner material without silently verifying it.
    ///
    /// # Errors
    /// Returns `CaptureError` when the interview is complete, the answer is blank, or profile mutation fails.
    pub fn submit_answer(&mut self, answer: impl Into<String>) -> Result<(), CaptureError> {
        let answer = answer.into();
        if answer.trim().is_empty() {
            return Err(CaptureError::BlankAnswer);
        }
        let question = self
            .current_question()
            .ok_or(CaptureError::InterviewAlreadyComplete)?;
        let claim_id = question.claim_id().clone();
        let kind = question.kind();
        let record = OwnerClaimRecord::capture(
            claim_id,
            OwnerClaim {
                statement: answer,
                kind,
                source: SourceKind::Owner,
                verification: VerificationState::Unverified,
                derivation: DerivationKind::Direct,
            },
        )?;
        self.profile.add_captured_claim(record)?;
        self.next_question += 1;
        Ok(())
    }

    /// Finalizes answer capture while leaving all claims unverified until explicit review.
    ///
    /// # Errors
    /// Returns `CaptureError::InterviewIncomplete` until every planned question has an answer.
    pub fn finish_capture(&mut self) -> Result<(), CaptureError> {
        if self.next_question != self.plan.question_count() {
            return Err(CaptureError::InterviewIncomplete);
        }
        self.profile.mark_capture_complete()?;
        Ok(())
    }

    /// Explicitly approves one captured claim during owner review.
    ///
    /// # Errors
    /// Returns `CaptureError` when the claim is unavailable or the profile is not reviewable.
    pub fn approve_claim(&mut self, id: &ClaimId) -> Result<(), CaptureError> {
        self.profile.approve_claim(id)?;
        Ok(())
    }

    /// Corrects one claim while preserving its previous revisions.
    ///
    /// # Errors
    /// Returns `CaptureError` when correction is invalid or the claim cannot be changed now.
    pub fn correct_claim(
        &mut self,
        id: &ClaimId,
        statement: impl Into<String>,
        kind: ClaimKind,
    ) -> Result<(), CaptureError> {
        self.profile.correct_claim(id, statement, kind)?;
        Ok(())
    }

    /// Approves the initial identity/attribution boundary after every claim was reviewed.
    ///
    /// # Errors
    /// Returns `CaptureError` if claims remain unreviewed or the profile is in the wrong state.
    pub fn complete_initial_review(&mut self) -> Result<(), CaptureError> {
        self.profile.approve_initial_review()?;
        Ok(())
    }

    #[must_use]
    pub fn into_profile(self) -> PersonaProfile {
        self.profile
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureError {
    UnsupportedPersonaMode,
    ProfileNotCleanDraft,
    BlankAnswer,
    InterviewAlreadyComplete,
    InterviewIncomplete,
    Profile(ProfileError),
}

impl Display for CaptureError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedPersonaMode => {
                formatter.write_str("RT0 guided capture supports DIGITAL_TWIN personas only")
            }
            Self::ProfileNotCleanDraft => {
                formatter.write_str("guided capture requires one clean canonical Persona draft")
            }
            Self::BlankAnswer => formatter.write_str("guided capture answer must not be blank"),
            Self::InterviewAlreadyComplete => {
                formatter.write_str("guided interview is already complete")
            }
            Self::InterviewIncomplete => {
                formatter.write_str("guided interview still has unanswered questions")
            }
            Self::Profile(error) => Display::fmt(error, formatter),
        }
    }
}

impl Error for CaptureError {}

impl From<ProfileError> for CaptureError {
    fn from(error: ProfileError) -> Self {
        Self::Profile(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InterviewQuestion;
    use vpr_domain::{
        ConstitutionBoundary, PersonaId, PersonaIdentity, PersonaMode, PersonaVersion,
        VerifiedOwnerOpinion,
    };

    fn profile(mode: PersonaMode) -> PersonaProfile {
        PersonaProfile::new(
            PersonaIdentity::new(
                PersonaId::new("persona-1").unwrap(),
                PersonaVersion::new(1).unwrap(),
                mode,
            ),
            ConstitutionBoundary::strict_digital_twin(),
        )
    }

    fn plan() -> InterviewPlan {
        InterviewPlan::new(vec![
            InterviewQuestion::new(
                ClaimId::new("identity-self-description").unwrap(),
                "Как вы обычно представляете себя?",
                ClaimKind::Factual,
            )
            .unwrap(),
            InterviewQuestion::new(
                ClaimId::new("opinion-working-style").unwrap(),
                "Какой стиль работы вам ближе?",
                ClaimKind::Opinion,
            )
            .unwrap(),
        ])
        .unwrap()
    }

    #[test]
    fn guided_capture_requires_digital_twin() {
        assert_eq!(
            GuidedOwnerInterview::new(profile(PersonaMode::Expert), plan()).unwrap_err(),
            CaptureError::UnsupportedPersonaMode
        );
    }

    #[test]
    fn capture_does_not_silently_verify_owner_material() {
        let mut interview =
            GuidedOwnerInterview::new(profile(PersonaMode::DigitalTwin), plan()).unwrap();
        interview.submit_answer("Сергей, предприниматель").unwrap();
        let id = ClaimId::new("identity-self-description").unwrap();
        let claim = interview.profile().claim(&id).unwrap().current().claim();
        assert_eq!(claim.verification, VerificationState::Unverified);
    }

    #[test]
    fn full_review_makes_verified_owner_opinion_eligible() {
        let mut interview =
            GuidedOwnerInterview::new(profile(PersonaMode::DigitalTwin), plan()).unwrap();
        interview.submit_answer("Сергей, предприниматель").unwrap();
        interview.submit_answer("Люблю быстрые итерации").unwrap();
        interview.finish_capture().unwrap();

        let factual = ClaimId::new("identity-self-description").unwrap();
        let opinion = ClaimId::new("opinion-working-style").unwrap();
        interview.approve_claim(&factual).unwrap();
        interview.approve_claim(&opinion).unwrap();
        interview.complete_initial_review().unwrap();

        assert_eq!(
            interview.profile().capture_state(),
            PersonaCaptureState::Reviewed
        );
        assert_eq!(interview.profile().identity().version().get(), 2);
        let claim = interview
            .profile()
            .claim(&opinion)
            .unwrap()
            .current()
            .claim()
            .clone();
        assert!(VerifiedOwnerOpinion::try_from(claim).is_ok());
    }

    #[test]
    fn cannot_finalize_incomplete_interview() {
        let mut interview =
            GuidedOwnerInterview::new(profile(PersonaMode::DigitalTwin), plan()).unwrap();
        interview.submit_answer("Сергей").unwrap();
        assert_eq!(
            interview.finish_capture(),
            Err(CaptureError::InterviewIncomplete)
        );
    }
}
