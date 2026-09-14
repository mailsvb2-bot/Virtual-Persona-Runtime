use std::error::Error;
use std::fmt::{Display, Formatter};

use serde::Serialize;
use vpr_capture::{CaptureError, GuidedOwnerInterview, InterviewPlan, InterviewQuestion, PlanError};
use vpr_domain::{
    ClaimId, ClaimKind, ConstitutionBoundary, PersonaCaptureState, PersonaId, PersonaIdentity,
    PersonaMode, PersonaProfile, PersonaVersion, VerificationState,
};

#[derive(Debug)]
pub struct Rt0OwnerCapture {
    interview: GuidedOwnerInterview,
}

impl Rt0OwnerCapture {
    /// Creates the bounded RT0 guided capture over one canonical `DIGITAL_TWIN` profile.
    ///
    /// # Errors
    /// Returns a typed setup error if the fixed RT0 interview plan or profile cannot be built.
    pub fn new(persona_id: PersonaId) -> Result<Self, OwnerCaptureError> {
        let version = PersonaVersion::new(1).ok_or(OwnerCaptureError::VersionUnavailable)?;
        let profile = PersonaProfile::new(
            PersonaIdentity::new(persona_id, version, PersonaMode::DigitalTwin),
            ConstitutionBoundary::strict_digital_twin(),
        );
        let interview = GuidedOwnerInterview::new(profile, rt0_interview_plan()?)?;
        Ok(Self { interview })
    }

    #[must_use]
    pub fn snapshot(&self) -> OwnerCaptureSnapshot {
        let profile = self.interview.profile();
        let current_question = self
            .interview
            .current_question()
            .map(|question| OwnerCaptureQuestion {
                claim_id: question.claim_id().as_str().to_owned(),
                prompt: question.prompt().to_owned(),
                kind: claim_kind_name(question.kind()).to_owned(),
            });
        let claims = profile
            .claims()
            .iter()
            .map(|record| {
                let current = record.current();
                let claim = current.claim();
                OwnerCaptureClaim {
                    claim_id: record.id().as_str().to_owned(),
                    statement: claim.statement.clone(),
                    kind: claim_kind_name(claim.kind).to_owned(),
                    verification: verification_name(claim.verification).to_owned(),
                    revision: current.revision().get(),
                    owner_reviewed: record.is_owner_reviewed(),
                }
            })
            .collect();
        OwnerCaptureSnapshot {
            persona_id: profile.identity().id().as_str().to_owned(),
            persona_version: profile.identity().version().get(),
            capture_state: capture_state_name(profile.capture_state()).to_owned(),
            answered_count: self.interview.answered_count(),
            question_count: self.interview.question_count(),
            current_question,
            claims,
        }
    }

    /// Captures one direct owner answer without silently verifying it.
    ///
    /// # Errors
    /// Returns the canonical capture error for invalid input or state.
    pub fn submit_answer(&mut self, answer: impl Into<String>) -> Result<(), OwnerCaptureError> {
        self.interview.submit_answer(answer)?;
        Ok(())
    }

    /// Marks answer capture complete while preserving explicit owner review as a separate step.
    ///
    /// # Errors
    /// Returns the canonical capture error until every RT0 question is answered.
    pub fn finish_capture(&mut self) -> Result<(), OwnerCaptureError> {
        self.interview.finish_capture()?;
        Ok(())
    }

    /// Explicitly approves one captured owner claim.
    ///
    /// # Errors
    /// Returns the canonical capture error when review is not allowed or the claim is absent.
    pub fn approve_claim(&mut self, id: &ClaimId) -> Result<(), OwnerCaptureError> {
        self.interview.approve_claim(id)?;
        Ok(())
    }

    /// Explicitly corrects one captured claim while preserving its revision history.
    ///
    /// # Errors
    /// Returns the canonical capture error when the correction is not allowed.
    pub fn correct_claim(
        &mut self,
        id: &ClaimId,
        statement: impl Into<String>,
        kind: ClaimKind,
    ) -> Result<(), OwnerCaptureError> {
        self.interview.correct_claim(id, statement, kind)?;
        Ok(())
    }

    /// Finalizes the initial owner review without moving canonical profile ownership yet.
    ///
    /// # Errors
    /// Returns the canonical capture error until every claim is owner-reviewed.
    pub fn complete_initial_review(&mut self) -> Result<(), OwnerCaptureError> {
        self.interview.complete_initial_review()?;
        Ok(())
    }

    #[must_use]
    pub fn into_profile(self) -> PersonaProfile {
        self.interview.into_profile()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OwnerCaptureSnapshot {
    pub persona_id: String,
    pub persona_version: u64,
    pub capture_state: String,
    pub answered_count: usize,
    pub question_count: usize,
    pub current_question: Option<OwnerCaptureQuestion>,
    pub claims: Vec<OwnerCaptureClaim>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OwnerCaptureQuestion {
    pub claim_id: String,
    pub prompt: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OwnerCaptureClaim {
    pub claim_id: String,
    pub statement: String,
    pub kind: String,
    pub verification: String,
    pub revision: u64,
    pub owner_reviewed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerCaptureError {
    Plan(PlanError),
    Capture(CaptureError),
    InvalidStaticPlan,
    VersionUnavailable,
}

impl Display for OwnerCaptureError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plan(error) => Display::fmt(error, formatter),
            Self::Capture(error) => Display::fmt(error, formatter),
            Self::InvalidStaticPlan => formatter.write_str("RT0 owner capture plan is invalid"),
            Self::VersionUnavailable => formatter.write_str("initial PersonaVersion is unavailable"),
        }
    }
}

impl Error for OwnerCaptureError {}

impl From<PlanError> for OwnerCaptureError {
    fn from(error: PlanError) -> Self {
        Self::Plan(error)
    }
}

impl From<CaptureError> for OwnerCaptureError {
    fn from(error: CaptureError) -> Self {
        Self::Capture(error)
    }
}

fn rt0_interview_plan() -> Result<InterviewPlan, OwnerCaptureError> {
    InterviewPlan::new(vec![
        InterviewQuestion::new(
            static_claim_id("identity-self-description")?,
            "Как вы обычно представляете себя в двух-трёх предложениях?",
            ClaimKind::Factual,
        )?,
        InterviewQuestion::new(
            static_claim_id("preference-communication-style")?,
            "Как вам лучше отвечать людям: кратко или подробно, формально или неформально?",
            ClaimKind::Preference,
        )?,
        InterviewQuestion::new(
            static_claim_id("opinion-core-principle")?,
            "Какой принцип или взгляд особенно важно корректно передавать от вашего имени?",
            ClaimKind::Opinion,
        )?,
    ])
    .map_err(OwnerCaptureError::from)
}

fn static_claim_id(value: &'static str) -> Result<ClaimId, OwnerCaptureError> {
    ClaimId::new(value).map_err(|_| OwnerCaptureError::InvalidStaticPlan)
}

const fn claim_kind_name(kind: ClaimKind) -> &'static str {
    match kind {
        ClaimKind::Factual => "factual",
        ClaimKind::Opinion => "opinion",
        ClaimKind::Preference => "preference",
        ClaimKind::Prediction => "prediction",
        ClaimKind::ValueJudgment => "value_judgment",
    }
}

const fn verification_name(state: VerificationState) -> &'static str {
    match state {
        VerificationState::Unverified => "unverified",
        VerificationState::Corroborated => "corroborated",
        VerificationState::OwnerVerified => "owner_verified",
        VerificationState::SourceVerified => "source_verified",
        VerificationState::Disputed => "disputed",
    }
}

const fn capture_state_name(state: PersonaCaptureState) -> &'static str {
    match state {
        PersonaCaptureState::Draft => "draft",
        PersonaCaptureState::Captured => "captured",
        PersonaCaptureState::Reviewed => "reviewed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture() -> Rt0OwnerCapture {
        Rt0OwnerCapture::new(PersonaId::new("owner-capture-persona").unwrap()).unwrap()
    }

    #[test]
    fn snapshot_exposes_bounded_question_without_verifying_answers() {
        let mut capture = capture();
        let initial = capture.snapshot();
        assert_eq!(initial.capture_state, "draft");
        assert_eq!(initial.question_count, 3);
        assert_eq!(
            initial.current_question.unwrap().claim_id,
            "identity-self-description"
        );

        capture.submit_answer("Сергей, предприниматель").unwrap();
        let snapshot = capture.snapshot();
        assert_eq!(snapshot.answered_count, 1);
        assert_eq!(snapshot.claims.len(), 1);
        assert_eq!(snapshot.claims[0].verification, "unverified");
        assert!(!snapshot.claims[0].owner_reviewed);
    }

    #[test]
    fn explicit_review_produces_one_reviewed_versioned_profile() {
        let mut capture = capture();
        capture.submit_answer("Сергей, предприниматель").unwrap();
        capture.submit_answer("Кратко и по существу").unwrap();
        capture.submit_answer("Проверять результат маленькими шагами").unwrap();
        capture.finish_capture().unwrap();

        let identity = ClaimId::new("identity-self-description").unwrap();
        let style = ClaimId::new("preference-communication-style").unwrap();
        let principle = ClaimId::new("opinion-core-principle").unwrap();
        capture.approve_claim(&identity).unwrap();
        capture.approve_claim(&style).unwrap();
        capture
            .correct_claim(
                &principle,
                "Предпочитаю короткие циклы проверки",
                ClaimKind::Opinion,
            )
            .unwrap();
        capture.complete_initial_review().unwrap();

        let snapshot = capture.snapshot();
        assert_eq!(snapshot.capture_state, "reviewed");
        assert_eq!(snapshot.persona_version, 2);
        assert!(snapshot.claims.iter().all(|claim| claim.owner_reviewed));
        let profile = capture.into_profile();
        assert_eq!(profile.capture_state(), PersonaCaptureState::Reviewed);
        assert_eq!(profile.identity().version().get(), 2);
    }
}
