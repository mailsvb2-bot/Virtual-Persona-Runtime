use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::{
    ClaimId, ClaimKind, ConstitutionBoundary, DerivationKind, MAX_OWNER_CLAIM_CHARS, OwnerClaim,
    PersonaIdentity, PersonaVersionExhausted, SourceKind, VerificationState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClaimRevision(u64);

impl ClaimRevision {
    #[must_use]
    pub const fn initial() -> Self {
        Self(1)
    }

    #[must_use]
    pub fn new(value: u64) -> Option<Self> {
        (value > 0).then_some(Self(value))
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

    #[must_use]
    pub fn retained_revisions(&self) -> impl Iterator<Item = &OwnerClaimRevision> {
        self.previous_revisions
            .iter()
            .chain(std::iter::once(&self.current))
    }

    #[must_use]
    pub fn has_complete_history(&self) -> bool {
        self.retained_revisions()
            .next()
            .is_some_and(|revision| revision.revision() == ClaimRevision::initial())
    }

    /// Restores retained claim history without fabricating revisions that are not present.
    ///
    /// A complete history starts at revision 1 with direct unverified owner material. A partial
    /// legacy history may start later, but every retained revision must be contiguous direct owner
    /// material and the current revision must be owner-verified.
    ///
    /// # Errors
    /// Returns `ProfileError` when history is empty, non-contiguous, malformed, or not reviewed.
    pub fn restore_retained_history(
        id: ClaimId,
        revisions: Vec<(u64, OwnerClaim)>,
    ) -> Result<Self, ProfileError> {
        let mut restored = Vec::with_capacity(revisions.len());
        let mut previous_revision: Option<ClaimRevision> = None;
        let starts_at_initial = revisions
            .first()
            .is_some_and(|(revision, _)| *revision == ClaimRevision::initial().get());

        for (index, (revision_number, claim)) in revisions.into_iter().enumerate() {
            let revision =
                ClaimRevision::new(revision_number).ok_or(ProfileError::InvalidClaimHistory)?;
            if let Some(previous) = previous_revision
                && previous.next()? != revision
            {
                return Err(ProfileError::InvalidClaimHistory);
            }
            validate_restored_claim(&claim, index, starts_at_initial)?;
            previous_revision = Some(revision);
            restored.push(OwnerClaimRevision { revision, claim });
        }

        let current = restored.pop().ok_or(ProfileError::InvalidClaimHistory)?;
        if current.claim.verification != VerificationState::OwnerVerified {
            return Err(ProfileError::InvalidClaimHistory);
        }
        Ok(Self {
            id,
            previous_revisions: restored,
            current,
        })
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
        if statement.chars().count() > MAX_OWNER_CLAIM_CHARS {
            return Err(ProfileError::ClaimStatementTooLong);
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

#[derive(Debug, PartialEq, Eq)]
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

    /// Restores a reviewed profile from validated retained claim histories.
    ///
    /// This constructor never replays or fabricates missing revisions. Partial legacy histories
    /// remain explicitly partial inside their `OwnerClaimRecord`.
    ///
    /// # Errors
    /// Returns `ProfileError` when the profile version cannot represent a reviewed profile, claims
    /// are empty or duplicated, or any current claim is not direct owner-verified material.
    pub fn restore_reviewed(
        identity: PersonaIdentity,
        constitution: ConstitutionBoundary,
        claims: Vec<OwnerClaimRecord>,
    ) -> Result<Self, ProfileError> {
        if identity.version().get() < 2 {
            return Err(ProfileError::InvalidReviewedProfileVersion);
        }
        if claims.is_empty() {
            return Err(ProfileError::EmptyCapture);
        }
        if claims.iter().any(|record| !record.is_owner_reviewed()) {
            return Err(ProfileError::ClaimsNotReviewed);
        }
        for (index, record) in claims.iter().enumerate() {
            if claims[..index]
                .iter()
                .any(|existing| existing.id() == record.id())
            {
                return Err(ProfileError::DuplicateClaimId);
            }
        }
        Ok(Self {
            identity,
            constitution,
            capture_state: PersonaCaptureState::Reviewed,
            claims,
        })
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
        let next_version = if was_reviewed {
            Some(self.identity.version().next()?)
        } else {
            None
        };
        let record = self.claim_mut(id).ok_or(ProfileError::ClaimNotFound)?;
        record.correct(statement, kind)?;
        if let Some(version) = next_version {
            self.identity.apply_version(version);
        }
        Ok(())
    }

    /// Corrects one claim and lets the caller atomically commit the resulting profile.
    ///
    /// Only the touched current revision, its prior history length, and previous `PersonaVersion`
    /// are retained for rollback; history is never cloned. The canonical `PersonaProfile` itself
    /// remains non-cloneable. If the commit callback fails, the exact claim revision history and
    /// `PersonaVersion` are restored before the error returns.
    ///
    /// # Errors
    /// Returns a correction error when the mutation is invalid, or the caller's commit error after
    /// restoring the exact pre-correction domain state.
    pub fn correct_claim_transactional<E>(
        &mut self,
        id: &ClaimId,
        statement: impl Into<String>,
        kind: ClaimKind,
        commit: impl FnOnce(&Self) -> Result<(), E>,
    ) -> Result<(), TransactionalCorrectionError<E>> {
        let record_index = self
            .claims
            .iter()
            .position(|record| record.id() == id)
            .ok_or(TransactionalCorrectionError::Correction(
                ProfileError::ClaimNotFound,
            ))?;
        let previous_current = self.claims[record_index].current.clone();
        let previous_history_len = self.claims[record_index].previous_revisions.len();
        let previous_version = self.identity.version();

        self.correct_claim(id, statement, kind)
            .map_err(TransactionalCorrectionError::Correction)?;

        if let Err(error) = commit(self) {
            let record = &mut self.claims[record_index];
            record.current = previous_current;
            record.previous_revisions.truncate(previous_history_len);
            self.identity.apply_version(previous_version);
            return Err(TransactionalCorrectionError::Commit(error));
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
        let next_version = self.identity.version().next()?;
        self.identity.apply_version(next_version);
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

#[derive(Debug, PartialEq, Eq)]
pub enum TransactionalCorrectionError<E> {
    Correction(ProfileError),
    Commit(E),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileError {
    InvalidClaimProvenance,
    BlankClaimStatement,
    ClaimStatementTooLong,
    DuplicateClaimId,
    EmptyCapture,
    ClaimNotFound,
    ClaimsNotReviewed,
    InvalidCaptureState,
    InvalidClaimHistory,
    InvalidReviewedProfileVersion,
    ClaimRevisionExhausted,
    PersonaVersionExhausted,
}

impl Display for ProfileError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidClaimProvenance => "captured claim must be direct owner material",
            Self::BlankClaimStatement => "owner claim statement must not be blank",
            Self::ClaimStatementTooLong => "owner claim statement exceeds the canonical size limit",
            Self::DuplicateClaimId => "claim identity already exists in this persona",
            Self::EmptyCapture => "guided capture cannot be completed without claims",
            Self::ClaimNotFound => "claim was not found in this persona",
            Self::ClaimsNotReviewed => "all captured claims must be explicitly owner-reviewed",
            Self::InvalidCaptureState => "operation is not allowed in the current capture state",
            Self::InvalidClaimHistory => "retained claim history is invalid",
            Self::InvalidReviewedProfileVersion => "reviewed persona version is invalid",
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

fn validate_restored_claim(
    claim: &OwnerClaim,
    index: usize,
    starts_at_initial: bool,
) -> Result<(), ProfileError> {
    if claim.statement.trim().is_empty()
        || claim.statement.chars().count() > MAX_OWNER_CLAIM_CHARS
        || claim.source != SourceKind::Owner
        || claim.derivation != DerivationKind::Direct
    {
        return Err(ProfileError::InvalidClaimHistory);
    }
    let expected_verification = if starts_at_initial && index == 0 {
        VerificationState::Unverified
    } else {
        VerificationState::OwnerVerified
    };
    if claim.verification != expected_verification {
        return Err(ProfileError::InvalidClaimHistory);
    }
    Ok(())
}
fn validate_captured_claim(claim: &OwnerClaim) -> Result<(), ProfileError> {
    if claim.statement.trim().is_empty() {
        return Err(ProfileError::BlankClaimStatement);
    }
    if claim.statement.chars().count() > MAX_OWNER_CLAIM_CHARS {
        return Err(ProfileError::ClaimStatementTooLong);
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
mod tests;
