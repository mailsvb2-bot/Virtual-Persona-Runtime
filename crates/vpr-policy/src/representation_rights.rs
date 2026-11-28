//! Scoped representation rights. Persist only in the canonical owner record.
//! This is an authorization prerequisite, not a publication or session owner.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepresentationKind {
    Voice,
    Face,
    Body,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RightsBasis {
    VerifiedSelf,
    VerifiedLicense,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RightsUse {
    Preparation,
    PrivatePreview,
    PublicInteraction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepresentationGrant {
    pub owner_id: String,
    pub persona_id: String,
    pub representation_id: String,
    pub kind: RepresentationKind,
    pub basis: RightsBasis,
    pub evidence_ref: String,
    pub uses: BTreeSet<RightsUse>,
    pub commercial: bool,
    pub territories: Option<BTreeSet<String>>,
    pub expires_at_millis: Option<u64>,
    pub revoked: bool,
}

pub struct RightsRequest<'a> {
    pub owner_id: &'a str,
    pub persona_id: &'a str,
    pub representation_id: &'a str,
    pub kind: RepresentationKind,
    pub usage: RightsUse,
    pub commercial: bool,
    pub territory: Option<&'a str>,
    pub now_millis: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightsDenial {
    InvalidGrant,
    IdentityMismatch,
    Revoked,
    Expired,
    UseDenied,
    CommercialDenied,
    TerritoryDenied,
}

impl RepresentationGrant {
    /// Evaluate an independently verified grant against trusted request facts.
    /// No client-provided claim of ownership constitutes verification.
    ///
    /// # Errors
    /// Denies malformed, mismatched, revoked, expired or out-of-scope grants.
    pub fn authorize(&self, request: &RightsRequest<'_>) -> Result<(), RightsDenial> {
        if self.owner_id.trim().is_empty()
            || self.persona_id.trim().is_empty()
            || self.representation_id.trim().is_empty()
            || self.evidence_ref.trim().is_empty()
            || self.uses.is_empty()
        {
            return Err(RightsDenial::InvalidGrant);
        }
        if self.owner_id != request.owner_id
            || self.persona_id != request.persona_id
            || self.representation_id != request.representation_id
            || self.kind != request.kind
        {
            return Err(RightsDenial::IdentityMismatch);
        }
        if self.revoked {
            return Err(RightsDenial::Revoked);
        }
        if self
            .expires_at_millis
            .is_some_and(|expiry| request.now_millis >= expiry)
        {
            return Err(RightsDenial::Expired);
        }
        if !self.uses.contains(&request.usage) {
            return Err(RightsDenial::UseDenied);
        }
        if request.commercial && !self.commercial {
            return Err(RightsDenial::CommercialDenied);
        }
        if let Some(territories) = &self.territories
            && !request
                .territory
                .is_some_and(|territory| territories.contains(territory))
        {
            return Err(RightsDenial::TerritoryDenied);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_scope_and_revocation_are_enforced() {
        let mut grant = RepresentationGrant {
            owner_id: "owner-a".into(),
            persona_id: "persona-a".into(),
            representation_id: "voice-a".into(),
            kind: RepresentationKind::Voice,
            basis: RightsBasis::VerifiedSelf,
            evidence_ref: "verified-evidence".into(),
            uses: BTreeSet::from([RightsUse::PublicInteraction]),
            commercial: false,
            territories: Some(BTreeSet::from(["SE".into()])),
            expires_at_millis: Some(100),
            revoked: false,
        };
        let mut request = RightsRequest {
            owner_id: "owner-a",
            persona_id: "persona-a",
            representation_id: "voice-a",
            kind: RepresentationKind::Voice,
            usage: RightsUse::PublicInteraction,
            commercial: false,
            territory: Some("SE"),
            now_millis: 99,
        };
        assert_eq!(grant.authorize(&request), Ok(()));
        request.owner_id = "owner-b";
        assert_eq!(
            grant.authorize(&request),
            Err(RightsDenial::IdentityMismatch)
        );
        request.owner_id = "owner-a";
        request.commercial = true;
        assert_eq!(
            grant.authorize(&request),
            Err(RightsDenial::CommercialDenied)
        );
        request.commercial = false;
        request.territory = None;
        assert_eq!(
            grant.authorize(&request),
            Err(RightsDenial::TerritoryDenied)
        );
        request.territory = Some("SE");
        request.now_millis = 100;
        assert_eq!(grant.authorize(&request), Err(RightsDenial::Expired));
        request.now_millis = 99;
        grant.revoked = true;
        assert_eq!(grant.authorize(&request), Err(RightsDenial::Revoked));
    }
}
