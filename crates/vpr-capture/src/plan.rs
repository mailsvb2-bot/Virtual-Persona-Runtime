use std::collections::HashSet;
use std::error::Error;
use std::fmt::{Display, Formatter};

use vpr_domain::{ClaimId, ClaimKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterviewQuestion {
    claim_id: ClaimId,
    prompt: String,
    kind: ClaimKind,
}

impl InterviewQuestion {
    /// Creates one guided-capture question bound to a canonical claim identity.
    ///
    /// # Errors
    /// Returns `PlanError::BlankPrompt` when the prompt contains no visible text.
    pub fn new(
        claim_id: ClaimId,
        prompt: impl Into<String>,
        kind: ClaimKind,
    ) -> Result<Self, PlanError> {
        let prompt = prompt.into();
        if prompt.trim().is_empty() {
            return Err(PlanError::BlankPrompt);
        }
        Ok(Self {
            claim_id,
            prompt,
            kind,
        })
    }

    #[must_use]
    pub const fn claim_id(&self) -> &ClaimId {
        &self.claim_id
    }

    #[must_use]
    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    #[must_use]
    pub const fn kind(&self) -> ClaimKind {
        self.kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterviewPlan {
    questions: Vec<InterviewQuestion>,
}

impl InterviewPlan {
    /// Creates a non-empty guided interview with unique canonical claim identities.
    ///
    /// # Errors
    /// Returns `PlanError` when the plan is empty or reuses a claim identity.
    pub fn new(questions: Vec<InterviewQuestion>) -> Result<Self, PlanError> {
        if questions.is_empty() {
            return Err(PlanError::EmptyPlan);
        }
        let mut seen = HashSet::with_capacity(questions.len());
        for question in &questions {
            if !seen.insert(question.claim_id().clone()) {
                return Err(PlanError::DuplicateClaimId);
            }
        }
        Ok(Self { questions })
    }

    #[must_use]
    pub fn questions(&self) -> &[InterviewQuestion] {
        &self.questions
    }

    #[must_use]
    pub fn question_count(&self) -> usize {
        self.questions.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanError {
    EmptyPlan,
    BlankPrompt,
    DuplicateClaimId,
}

impl Display for PlanError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::EmptyPlan => "guided owner interview plan must contain at least one question",
            Self::BlankPrompt => "guided owner interview prompt must not be blank",
            Self::DuplicateClaimId => "guided owner interview cannot reuse a claim identity",
        })
    }
}

impl Error for PlanError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_rejects_duplicate_claim_identity() {
        let id = ClaimId::new("identity-self-description").unwrap();
        let first = InterviewQuestion::new(id.clone(), "Кто вы?", ClaimKind::Factual).unwrap();
        let second = InterviewQuestion::new(id, "Расскажите о себе", ClaimKind::Factual).unwrap();
        assert_eq!(
            InterviewPlan::new(vec![first, second]),
            Err(PlanError::DuplicateClaimId)
        );
    }
}
