use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimKind {
    Factual,
    Opinion,
    Preference,
    Prediction,
    ValueJudgment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Owner,
    User,
    Document,
    Web,
    Tool,
    Model,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationState {
    Unverified,
    Corroborated,
    OwnerVerified,
    SourceVerified,
    Disputed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DerivationKind {
    Direct,
    Remembered,
    Inferred,
    Summarized,
    Simulated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerClaim {
    pub statement: String,
    pub kind: ClaimKind,
    pub source: SourceKind,
    pub verification: VerificationState,
    pub derivation: DerivationKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedOwnerOpinion(OwnerClaim);

impl VerifiedOwnerOpinion {
    #[must_use]
    pub fn claim(&self) -> &OwnerClaim {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributionError {
    NotOpinion,
    NotOwnerSource,
    NotOwnerVerified,
    NotDirectOwnerEvidence,
}

impl Display for AttributionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NotOpinion => "claim is not an opinion",
            Self::NotOwnerSource => "opinion is not sourced directly from the owner",
            Self::NotOwnerVerified => "owner opinion is not owner-verified",
            Self::NotDirectOwnerEvidence => {
                "owner opinion is inferred, summarized, remembered or simulated"
            }
        })
    }
}

impl Error for AttributionError {}

impl TryFrom<OwnerClaim> for VerifiedOwnerOpinion {
    type Error = AttributionError;

    fn try_from(claim: OwnerClaim) -> Result<Self, Self::Error> {
        if claim.kind != ClaimKind::Opinion {
            return Err(AttributionError::NotOpinion);
        }
        if claim.source != SourceKind::Owner {
            return Err(AttributionError::NotOwnerSource);
        }
        if claim.verification != VerificationState::OwnerVerified {
            return Err(AttributionError::NotOwnerVerified);
        }
        if claim.derivation != DerivationKind::Direct {
            return Err(AttributionError::NotDirectOwnerEvidence);
        }
        Ok(Self(claim))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opinion(
        source: SourceKind,
        verification: VerificationState,
        derivation: DerivationKind,
    ) -> OwnerClaim {
        OwnerClaim {
            statement: "Мне нравится этот подход".into(),
            kind: ClaimKind::Opinion,
            source,
            verification,
            derivation,
        }
    }

    #[test]
    fn direct_owner_verified_opinion_is_eligible() {
        let claim = opinion(
            SourceKind::Owner,
            VerificationState::OwnerVerified,
            DerivationKind::Direct,
        );
        assert!(VerifiedOwnerOpinion::try_from(claim).is_ok());
    }

    #[test]
    fn model_inference_cannot_become_verified_owner_opinion() {
        let claim = opinion(
            SourceKind::Model,
            VerificationState::Unverified,
            DerivationKind::Inferred,
        );
        assert_eq!(
            VerifiedOwnerOpinion::try_from(claim),
            Err(AttributionError::NotOwnerSource)
        );
    }

    #[test]
    fn simulated_owner_text_cannot_become_verified_owner_opinion() {
        let claim = opinion(
            SourceKind::Owner,
            VerificationState::OwnerVerified,
            DerivationKind::Simulated,
        );
        assert_eq!(
            VerifiedOwnerOpinion::try_from(claim),
            Err(AttributionError::NotDirectOwnerEvidence)
        );
    }
}
