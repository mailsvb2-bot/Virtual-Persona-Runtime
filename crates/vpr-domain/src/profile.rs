use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::{
    ClaimId, ClaimKind, ConstitutionBoundary, DerivationKind, OwnerClaim, PersonaIdentity,
    PersonaVersionExhausted, SourceKind, VerificationState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClaimRevision(u64);

impl ClaimRevision {
    #[must_use]
    pub const fn initial() -> Self {
        Self(1)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns the next canonical claim revision.
    ///
    /// # Errors
    /// Returns `ClaimRevisionExhausted` if the revision cannot advance.
    pub fn next(self) -> Result<Self, ClaimRevisionExhausted> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(ClaimRevisionExhausted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimRevisionExhausted;

impl Display for ClaimRevisionExhausted {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("claim revision exhausted")
    }
}

impl Error for ClaimRevisionExhausted {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerClaimRevision {
    revision: ClaimRevision,
    claim: OwnerClaim,
}

impl OwnerClaimRevision {
    #[must_use]
    pub const fn revision(&self) -> ClaimRevision {
        self.revision
    }

    #[must_use]
    pub const fn claim(&self) -> &OwnerClaim {
        &self.claim
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerClaimRecord {
    id: ClaimId,
    previous_revisions: Vec<OwnerClaimRevision>,
    current: OwnerClaimRevision,
}

impl OwnerClaimRecord {
    /// Captures one direct, still-unreviewed owner claim.
    ///
    /// # Errors
    /// Returns `ProfileError` when provenance is not direct owner input or the statement is blank.
    pub fn capture(id: ClaimId, claim: OwnerClaim) -> Result<Self, ProfileError> {
        validate_captured_claim(&claim)?;
        Ok(Self {
            id,
            previous_revisions: Vec::new(),
            current: OwnerClaimRevision {
                revision: ClaimRevision::initial(),
                claim,
            },
        })
    }

    #[must_use]
    pub const fn id(&self) -> &ClaimId {
        &self.id
    }

    #[must_use]
    pub const fn current(&self) -> &OwnerClaimRevision {
        &self.current
    }

    #[must_use]
    pub fn previous_revisions(&self) -> &[OwnerClaimRevision] {
        &self.previous_revisions
    }

    #[must_use]
    pub fn is_owner_reviewed(&self) -> bool {
        let claim = self.current.claim();
        claim.source == SourceKind::Owner
            && claim.verification == VerificationState::OwnerVerified
            && claim.derivation == DerivationKind::Direct
    }

    /// Records explicit owner approval as a new claim revision.
    ///
    /// # Errors
    /// Returns `ProfileError` when the claim is not direct owner material or revisions are exhausted.
    pub fn approve(&mut self) -> Result<(), ProfileError> {
        if self.current.claim.source != SourceKind::Owner
            || self.current.claim.derivation != DerivationKind::Direct
        {
            return Err(ProfileError::InvalidClaimProvenance);
        }
        if self.current.claim.verification == VerificationState::OwnerVerified {
            return Ok(());
        }
        let mut approved = self.current.claim.clone();
        approved.verification = VerificationState::OwnerVerified;
        self.replace_current(approved)
    }

    /// Records an explicit owner correction as a new owner-verified revision.
    ///
    /// # Errors
    /// Returns `ProfileError` when the corrected statement is blank or revisions are exhausted.
    pub fn correct(
        &mut self,
        statement: impl Into<String>,
        kind: ClaimKind,
    ) -> Result<(), ProfileError> {
        let statement = statement.into();
        if statement.trim().is_empty() {
            return Err(ProfileError::BlankClaimStatement);
        }
        let corrected = OwnerClaim {
            statement,
            kind,
            source: SourceKind::Owner,
            verification: VerificationState::OwnerVerified,
            derivation: DerivationKind::Direct,
        };
        self.replace_current(corrected)
    }

    fn replace_current(&mut self, claim: OwnerClaim) -> Result<(), ProfileError> {
        let next_revision = self.current.revision.next()?;
        let replacement = OwnerClaimRevision {
            revision: next_revision,
            claim,
        };
        let previous = std::mem::replace(&mut self.current, replacement);
        self.previous_revisions.push(previous);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonaCaptureState {
    Draft,
    Captured,
    Reviewed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaProfile {
    identity: PersonaIdentity,
    constitution: ConstitutionBoundary,
    capture_state: PersonaCaptureState,
    claims: Vec<OwnerClaimRecord>,
}

impl PersonaProfile {
    #[must_use]
    pub const fn new(identity: PersonaIdentity, constitution: ConstitutionBoundary) -> Self {
        Self {
            identity,
            constitution,
            capture_state: PersonaCaptureState::Draft,
            claims: Vec::new(),
        }
    }

    #[must_use]
    pub const fn identity(&self) -> &PersonaIdentity {
        &self.identity
    }

    #[must_use]
    pub const fn constitution(&self) -> ConstitutionBoundary {
        self.constitution
    }

    #[must_use]
    pub const fn capture_state(&self) -> PersonaCaptureState {
        self.capture_state
    }

    #[must_use]
    pub fn claims(&self) -> &[OwnerClaimRecord] {
        &self.claims
    }

    #[must_use]
    pub fn claim(&self, id: &ClaimId) -> Option<&OwnerClaimRecord> {
        self.claims.iter().find(|record| record.id() == id)
    }

    /// Appends one captured claim before capture is finalized.
    ///
    /// # Errors
    /// Returns `ProfileError` for invalid state or duplicate claim identity.
    pub fn add_captured_claim(&mut self, record: OwnerClaimRecord) -> Result<(), ProfileError> {
        self.require_state(PersonaCaptureState::Draft)?;
        if self.claim(record.id()).is_some() {
            return Err(ProfileError::DuplicateClaimId);
        }
        self.claims.push(record);
        Ok(())
    }

    /// Marks the guided capture as complete without silently verifying its claims.
    ///
    /// # Errors
    /// Returns `ProfileError` if capture is empty or the profile is not a draft.
    pub fn mark_capture_complete(&mut self) -> Result<(), ProfileError> {
        self.require_state(PersonaCaptureState::Draft)?;
        if self.claims.is_empty() {
            return Err(ProfileError::EmptyCapture);
        }
        self.capture_state = PersonaCaptureState::Captured;
        Ok(())
    }

    /// Records explicit owner approval of one captured claim.
    ///
    /// # Errors
    /// Returns `ProfileError` when review is unavailable or the claim does not exist.
    pub fn approve_claim(&mut self, id: &ClaimId) -> Result<(), ProfileError> {
        self.require_state(PersonaCaptureState::Captured)?;
        let record = self.claim_mut(id).ok_or(ProfileError::ClaimNotFound)?;
        record.approve()
    }

    /// Records an owner correction while preserving prior claim revisions.
    ///
    /// # Errors
    /// Returns `ProfileError` for an invalid state, missing claim, blank correction, or exhausted version.
    pub fn correct_claim(
        &mut self,
        id: &ClaimId,
        statement: impl Into<String>,
        kind: ClaimKind,
    ) -> Result<(), ProfileError> {
        if !matches!(
            self.capture_state,
            PersonaCaptureState::Captured | PersonaCaptureState::Reviewed
        ) {
            return Err(ProfileError::InvalidCaptureState);
        }
        let was_reviewed = self.capture_state == PersonaCaptureState::Reviewed;
        let record = self.claim_mut(id).ok_or(ProfileError::ClaimNotFound)?;
        record.correct(statement, kind)?;
        if was_reviewed {
            self.identity.advance_version()?;
        }
        Ok(())
    }

    /// Approves the initial reviewed identity/attribution boundary and advances Persona version.
    ///
    /// # Errors
    /// Returns `ProfileError` unless every captured claim is explicitly owner-reviewed.
    pub fn approve_initial_review(&mut self) -> Result<(), ProfileError> {
        self.require_state(PersonaCaptureState::Captured)?;
        if !self.claims.iter().all(OwnerClaimRecord::is_owner_reviewed) {
            return Err(ProfileError::ClaimsNotReviewed);
        }
        self.identity.advance_version()?;
        self.capture_state = PersonaCaptureState::Reviewed;
        Ok(())
    }

    fn claim_mut(&mut self, id: &ClaimId) -> Option<&mut OwnerClaimRecord> {
        self.claims.iter_mut().find(|record| record.id() == id)
    }

    fn require_state(&self, expected: PersonaCaptureState) -> Result<(), ProfileError> {
        if self.capture_state != expected {
            return Err(ProfileError::InvalidCaptureState);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileError {
    InvalidClaimProvenance,
    BlankClaimStatement,
    DuplicateClaimId,
    EmptyCapture,
    ClaimNotFound,
    ClaimsNotReviewed,
    InvalidCaptureState,
    ClaimRevisionExhausted,
    PersonaVersionExhausted,
}

impl Display for ProfileError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidClaimProvenance => "captured claim must be direct owner material",
            Self::BlankClaimStatement => "owner claim statement must not be blank",
            Self::DuplicateClaimId => "claim identity already exists in this persona",
            Self::EmptyCapture => "guided capture cannot be completed without claims",
            Self::ClaimNotFound => "claim was not found in this persona",
            Self::ClaimsNotReviewed => "all captured claims must be explicitly owner-reviewed",
            Self::InvalidCaptureState => "operation is not allowed in the current capture state",
            Self::ClaimRevisionExhausted => "claim revision exhausted",
            Self::PersonaVersionExhausted => "persona version exhausted",
        })
    }
}

impl Error for ProfileError {}

impl From<ClaimRevisionExhausted> for ProfileError {
    fn from(_: ClaimRevisionExhausted) -> Self {
        Self::ClaimRevisionExhausted
    }
}

impl From<PersonaVersionExhausted> for ProfileError {
    fn from(_: PersonaVersionExhausted) -> Self {
        Self::PersonaVersionExhausted
    }
}

fn validate_captured_claim(claim: &OwnerClaim) -> Result<(), ProfileError> {
    if claim.statement.trim().is_empty() {
        return Err(ProfileError::BlankClaimStatement);
    }
    if claim.source != SourceKind::Owner
        || claim.verification != VerificationState::Unverified
        || claim.derivation != DerivationKind::Direct
    {
        return Err(ProfileError::InvalidClaimProvenance);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PersonaId, PersonaMode, PersonaVersion, VerifiedOwnerOpinion};

    fn profile() -> PersonaProfile {
        PersonaProfile::new(
            PersonaIdentity::new(
                PersonaId::new("persona-1").unwrap(),
                PersonaVersion::new(1).unwrap(),
                PersonaMode::DigitalTwin,
            ),
            ConstitutionBoundary::strict_digital_twin(),
        )
    }

    fn captured_opinion() -> OwnerClaimRecord {
        OwnerClaimRecord::capture(
            ClaimId::new("opinion-1").unwrap(),
            OwnerClaim {
                statement: "Мне нравится этот подход".into(),
                kind: ClaimKind::Opinion,
                source: SourceKind::Owner,
                verification: VerificationState::Unverified,
                derivation: DerivationKind::Direct,
            },
        )
        .unwrap()
    }

    #[test]
    fn captured_claim_requires_explicit_review_before_attribution() {
        let record = captured_opinion();
        assert!(VerifiedOwnerOpinion::try_from(record.current().claim().clone()).is_err());
    }

    #[test]
    fn initial_review_advances_persona_version_once() {
        let mut profile = profile();
        let id = ClaimId::new("opinion-1").unwrap();
        profile.add_captured_claim(captured_opinion()).unwrap();
        profile.mark_capture_complete().unwrap();
        profile.approve_claim(&id).unwrap();
        profile.approve_initial_review().unwrap();
        assert_eq!(profile.capture_state(), PersonaCaptureState::Reviewed);
        assert_eq!(profile.identity().version().get(), 2);
        let claim = profile.claim(&id).unwrap().current().claim().clone();
        assert!(VerifiedOwnerOpinion::try_from(claim).is_ok());
    }

    #[test]
    fn reviewed_correction_preserves_history_and_advances_persona_version() {
        let mut profile = profile();
        let id = ClaimId::new("opinion-1").unwrap();
        profile.add_captured_claim(captured_opinion()).unwrap();
        profile.mark_capture_complete().unwrap();
        profile.approve_claim(&id).unwrap();
        profile.approve_initial_review().unwrap();
        profile
            .correct_claim(&id, "Теперь я предпочитаю другой подход", ClaimKind::Opinion)
            .unwrap();

        let record = profile.claim(&id).unwrap();
        assert_eq!(record.current().revision().get(), 3);
        assert_eq!(record.previous_revisions().len(), 2);
        assert_eq!(profile.identity().version().get(), 3);
        assert_eq!(
            record.current().claim().statement,
            "Теперь я предпочитаю другой подход"
        );
    }
}
