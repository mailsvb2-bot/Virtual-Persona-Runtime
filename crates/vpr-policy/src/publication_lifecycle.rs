//! Canonical RT1 publication lifecycle, owned by the policy layer.
//!
//! No HTTP endpoint, browser token or public network binding is enabled here.
//! This is a bounded pre-entry domain capability. Persist this state alongside
//! the canonical Persona transaction before enabling public use.
use crate::publication_access::{
    PublicationAccess, PublicationAccessDenial, authorize_public_visitor,
};
use crate::{AuthorizationSnapshot, AuthorizationState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationStatus {
    Private,
    Published,
    Paused,
    Unpublished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationError {
    OwnerDenied,
    WrongPersona,
    WrongPublication,
    InvalidVersion,
    InvalidTransition,
    AuthEpochExhausted,
    VisitorDenied(PublicationAccessDenial),
}

/// Stable representation stored atomically with the canonical Persona.
/// It never contains a visitor credential or grants public access on its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalPublicationSnapshot {
    pub publication_id: String,
    pub owner_id: String,
    pub persona_id: String,
    pub persona_version: u64,
    pub status: PublicationStatus,
    pub authorization_epoch: u64,
    pub authorization_revoked: bool,
}

/// A permit belongs to exactly one canonical publication and Persona revision.
/// The caller must keep it server-side, not trust client-supplied fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisitorPermit {
    publication_id: String,
    owner_id: String,
    persona_id: String,
    persona_version: u64,
    snapshot: AuthorizationSnapshot,
}

impl VisitorPermit {
    /// A visitor permit is valid only for the exact reviewed Persona revision.
    #[must_use]
    pub fn matches_persona(&self, persona_id: &str, persona_version: u64) -> bool {
        self.persona_id == persona_id && self.persona_version == persona_version
    }
}

/// The owner and Persona identifiers MUST originate in a verified canonical
/// workspace transaction, never a URL, form field or unauthenticated token.
#[derive(Debug)]
pub struct CanonicalPublication {
    publication_id: String,
    owner_id: String,
    persona_id: String,
    persona_version: u64,
    status: PublicationStatus,
    authority: AuthorizationState,
}

impl CanonicalPublication {
    /// Create a private, unpublished policy projection of the canonical Persona.
    ///
    /// # Errors
    /// Rejects missing identity or zero `PersonaVersion`.
    pub fn new(
        owner_id: &str,
        persona_id: &str,
        publication_id: &str,
        version: u64,
    ) -> Result<Self, PublicationError> {
        if owner_id.trim().is_empty()
            || persona_id.trim().is_empty()
            || publication_id.trim().is_empty()
            || version == 0
        {
            return Err(PublicationError::InvalidVersion);
        }
        Ok(Self {
            publication_id: publication_id.to_owned(),
            owner_id: owner_id.to_owned(),
            persona_id: persona_id.to_owned(),
            persona_version: version,
            status: PublicationStatus::Private,
            authority: AuthorizationState::new(None),
        })
    }

    #[must_use]
    pub const fn status(&self) -> PublicationStatus {
        self.status
    }

    #[must_use]
    pub const fn persona_version(&self) -> u64 {
        self.persona_version
    }

    /// Return a versioned owner-publication record for the one canonical commit.
    #[must_use]
    pub fn durable_snapshot(&self) -> CanonicalPublicationSnapshot {
        CanonicalPublicationSnapshot {
            publication_id: self.publication_id.clone(),
            owner_id: self.owner_id.clone(),
            persona_id: self.persona_id.clone(),
            persona_version: self.persona_version,
            status: self.status,
            authorization_epoch: self.authority.epoch().get(),
            authorization_revoked: self.authority.is_revoked(),
        }
    }

    /// Restore canonical state from the same durable Persona transaction.
    /// Does not authorize traffic: the caller must first perform
    /// `invalidate_restart_leases`, durably commit its result and only
    /// then bind a network listener.
    ///
    /// # Errors
    /// Rejects malformed, contradictory or regressed publication data.
    pub fn from_durable_snapshot(
        state: CanonicalPublicationSnapshot,
    ) -> Result<Self, PublicationError> {
        if state.owner_id.trim().is_empty()
            || state.persona_id.trim().is_empty()
            || state.publication_id.trim().is_empty()
            || state.persona_version == 0
            || (state.status == PublicationStatus::Published && state.authorization_revoked)
            || (matches!(
                state.status,
                PublicationStatus::Paused | PublicationStatus::Unpublished
            ) && !state.authorization_revoked)
        {
            return Err(PublicationError::InvalidVersion);
        }
        let authority = AuthorizationState::from_persisted(
            state.authorization_epoch,
            state.authorization_revoked,
            None,
        )
        .ok_or(PublicationError::InvalidVersion)?;
        Ok(Self {
            publication_id: state.publication_id,
            owner_id: state.owner_id,
            persona_id: state.persona_id,
            persona_version: state.persona_version,
            status: state.status,
            authority,
        })
    }

    /// Invalidate every session permit issued by an earlier process lifetime.
    /// Caller must commit this epoch to durable storage before serving traffic.
    ///
    /// # Errors
    /// Fails closed on an exhausted authorization epoch.
    pub fn invalidate_restart_leases(&mut self) -> Result<(), PublicationError> {
        if self.status == PublicationStatus::Published {
            self.authority
                .replace(None)
                .map_err(|_| PublicationError::AuthEpochExhausted)?;
        } else {
            self.authority
                .revoke()
                .map_err(|_| PublicationError::AuthEpochExhausted)?;
        }
        Ok(())
    }

    /// Set published only after reviewed readiness/consent/cost checks supplied
    /// by canonical owners. Caller cannot change identity or version in place.
    ///
    /// # Errors
    /// Rejects unauthorized owner, stale or invalid versions, or invalid state.
    pub fn publish(
        &mut self,
        authenticated_owner: &str,
        current_persona_version: u64,
        readiness_authorized: bool,
    ) -> Result<(), PublicationError> {
        self.check_owner(authenticated_owner)?;
        if !readiness_authorized {
            return Err(PublicationError::OwnerDenied);
        }
        if current_persona_version != self.persona_version {
            return Err(PublicationError::InvalidVersion);
        }
        if !matches!(
            self.status,
            PublicationStatus::Private | PublicationStatus::Paused | PublicationStatus::Unpublished
        ) {
            return Err(PublicationError::InvalidTransition);
        }
        self.authority
            .replace(None)
            .map_err(|_| PublicationError::AuthEpochExhausted)?;
        self.status = PublicationStatus::Published;
        Ok(())
    }

    /// Explicitly terminate the previously issued visitor authority when paused.
    ///
    /// # Errors
    /// Rejects incorrect owner or an invalid transition.
    pub fn pause(&mut self, authenticated_owner: &str) -> Result<(), PublicationError> {
        self.check_owner(authenticated_owner)?;
        if self.status != PublicationStatus::Published {
            return Err(PublicationError::InvalidTransition);
        }
        self.revoke_authority()?;
        self.status = PublicationStatus::Paused;
        Ok(())
    }

    /// Unpublish without destroying the canonical Persona.
    ///
    /// # Errors
    /// Rejects wrong owner or invalid state.
    pub fn unpublish(&mut self, authenticated_owner: &str) -> Result<(), PublicationError> {
        self.check_owner(authenticated_owner)?;
        if matches!(
            self.status,
            PublicationStatus::Unpublished | PublicationStatus::Private
        ) {
            return Err(PublicationError::InvalidTransition);
        }
        self.revoke_authority()?;
        self.status = PublicationStatus::Unpublished;
        Ok(())
    }

    /// Owner correction invalidates the old published Persona version and all
    /// old visitor permits BEFORE a new publication can be created.
    ///
    /// # Errors
    /// Rejects version regression/zero, incorrect owner or exhausted epoch.
    pub fn correct_persona(
        &mut self,
        authenticated_owner: &str,
        new_version: u64,
    ) -> Result<(), PublicationError> {
        self.check_owner(authenticated_owner)?;
        if new_version <= self.persona_version {
            return Err(PublicationError::InvalidVersion);
        }
        self.revoke_authority()?;
        self.persona_version = new_version;
        self.status = PublicationStatus::Unpublished;
        Ok(())
    }

    /// Capture visitor authority for one particular published Persona version.
    /// Only an external authentication component may decide visitor identity.
    ///
    /// # Errors
    /// No visitor authority is minted for unpublished / paused personas.
    pub fn visitor_lease(
        &self,
        visitor_authenticated: bool,
    ) -> Result<VisitorPermit, PublicationError> {
        if !visitor_authenticated || self.status != PublicationStatus::Published {
            return Err(PublicationError::VisitorDenied(
                PublicationAccessDenial::Unauthenticated,
            ));
        }
        Ok(VisitorPermit {
            publication_id: self.publication_id.clone(),
            owner_id: self.owner_id.clone(),
            persona_id: self.persona_id.clone(),
            persona_version: self.persona_version,
            snapshot: self.authority.snapshot(),
        })
    }

    /// Validate a visitor permit against live policy state immediately before
    /// any media, text or provider operation.
    ///
    /// # Errors
    /// Returns a typed denial after pause, correction, unpublish, expiry,
    /// wrong workspace/Persona or revocation.
    pub fn authorize_visitor(
        &self,
        visitor_authenticated: bool,
        current_owner: &str,
        current_persona: &str,
        current_version: u64,
        permit: &VisitorPermit,
        now_millis: u64,
    ) -> Result<(), PublicationError> {
        if permit.publication_id != self.publication_id
            || permit.owner_id != self.owner_id
            || permit.persona_id != self.persona_id
            || permit.persona_version != self.persona_version
        {
            return Err(PublicationError::WrongPublication);
        }
        authorize_public_visitor(
            PublicationAccess {
                publication_owner: &self.owner_id,
                persona_owner: current_owner,
                published_persona: &self.persona_id,
                current_persona,
                published_persona_version: self.persona_version,
                current_persona_version: current_version,
                published: self.status == PublicationStatus::Published,
                visitor_authorized: visitor_authenticated,
                lease: permit.snapshot,
                now_millis,
            },
            &self.authority,
        )
        .map_err(PublicationError::VisitorDenied)
    }

    fn check_owner(&self, owner: &str) -> Result<(), PublicationError> {
        if owner != self.owner_id {
            return Err(PublicationError::OwnerDenied);
        }
        Ok(())
    }

    fn revoke_authority(&mut self) -> Result<(), PublicationError> {
        self.authority
            .revoke()
            .map_err(|_| PublicationError::AuthEpochExhausted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_to_visitor_to_pause_to_republish_rejects_stale_lease() {
        let mut p = CanonicalPublication::new("owner-a", "persona-a", "publication-a", 3).unwrap();
        assert_eq!(p.status(), PublicationStatus::Private);
        assert!(p.visitor_lease(true).is_err());
        assert_eq!(
            p.publish("owner-b", 3, true),
            Err(PublicationError::OwnerDenied)
        );
        assert_eq!(
            p.publish("owner-a", 3, false),
            Err(PublicationError::OwnerDenied)
        );
        p.publish("owner-a", 3, true).unwrap();
        let first = p.visitor_lease(true).unwrap();
        assert_eq!(
            p.authorize_visitor(true, "owner-a", "persona-a", 3, &first, 10),
            Ok(())
        );
        p.pause("owner-a").unwrap();
        assert!(
            p.authorize_visitor(true, "owner-a", "persona-a", 3, &first, 11)
                .is_err()
        );
        assert!(p.visitor_lease(true).is_err());
        p.publish("owner-a", 3, true).unwrap();
        assert!(
            p.authorize_visitor(true, "owner-a", "persona-a", 3, &first, 12)
                .is_err()
        );
        let current = p.visitor_lease(true).unwrap();
        assert_eq!(
            p.authorize_visitor(true, "owner-a", "persona-a", 3, &current, 13),
            Ok(())
        );
    }

    #[test]
    fn correction_invalidates_visitors_and_requires_explicit_republish() {
        let mut p = CanonicalPublication::new("owner-a", "persona-a", "publication-a", 4).unwrap();
        p.publish("owner-a", 4, true).unwrap();
        let prior = p.visitor_lease(true).unwrap();
        assert_eq!(
            p.correct_persona("owner-b", 5),
            Err(PublicationError::OwnerDenied)
        );
        assert_eq!(
            p.correct_persona("owner-a", 4),
            Err(PublicationError::InvalidVersion)
        );
        p.correct_persona("owner-a", 5).unwrap();
        assert_eq!(p.persona_version(), 5);
        assert_eq!(p.status(), PublicationStatus::Unpublished);
        assert!(
            p.authorize_visitor(true, "owner-a", "persona-a", 5, &prior, 20)
                .is_err()
        );
        assert_eq!(
            p.publish("owner-a", 4, true),
            Err(PublicationError::InvalidVersion)
        );
        p.publish("owner-a", 5, true).unwrap();
        assert!(
            p.authorize_visitor(true, "owner-a", "persona-a", 5, &prior, 20)
                .is_err()
        );
    }

    #[test]
    fn durable_json_roundtrip_and_unknown_fields_fail_closed() {
        let mut p = CanonicalPublication::new("owner-a", "persona-a", "publication-a", 7).unwrap();
        p.publish("owner-a", 7, true).unwrap();
        let json = serde_json::to_string(&p.durable_snapshot()).unwrap();
        let restored: CanonicalPublicationSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, p.durable_snapshot());
        let mut altered: serde_json::Value = serde_json::from_str(&json).unwrap();
        altered["unexpected_access"] = serde_json::json!(true);
        assert!(serde_json::from_value::<CanonicalPublicationSnapshot>(altered).is_err());
        let invalid = serde_json::json!({
            "publication_id":"publication-a",
            "owner_id":"owner-a",
            "persona_id":"persona-a",
            "persona_version":7,
            "status":"published",
            "authorization_epoch":0,
            "authorization_revoked":false
        });
        let parsed: CanonicalPublicationSnapshot = serde_json::from_value(invalid).unwrap();
        assert!(CanonicalPublication::from_durable_snapshot(parsed).is_err());
    }

    #[test]
    fn restart_recovery_invalidates_precrash_visitor_permits() {
        let mut p = CanonicalPublication::new("owner", "persona", "publication", 8).unwrap();
        p.publish("owner", 8, true).unwrap();
        let old = p.visitor_lease(true).unwrap();
        let durable = p.durable_snapshot();
        let mut reopened = CanonicalPublication::from_durable_snapshot(durable.clone()).unwrap();
        assert_eq!(reopened.durable_snapshot(), durable);
        reopened.invalidate_restart_leases().unwrap();
        assert_ne!(
            reopened.durable_snapshot().authorization_epoch,
            durable.authorization_epoch
        );
        assert_eq!(
            reopened.authorize_visitor(true, "owner", "persona", 8, &old, 9),
            Err(PublicationError::VisitorDenied(
                PublicationAccessDenial::StaleAuthorization
            ))
        );
        let fresh = reopened.visitor_lease(true).unwrap();
        assert_eq!(
            reopened.authorize_visitor(true, "owner", "persona", 8, &fresh, 9),
            Ok(())
        );
    }

    #[test]
    fn malformed_durable_owner_publication_state_denied() {
        let p = CanonicalPublication::new("owner", "persona", "publication", 1).unwrap();
        let snapshot = p.durable_snapshot();
        for malformed in [
            CanonicalPublicationSnapshot {
                authorization_epoch: 0,
                ..snapshot.clone()
            },
            CanonicalPublicationSnapshot {
                owner_id: String::new(),
                ..snapshot.clone()
            },
            CanonicalPublicationSnapshot {
                status: PublicationStatus::Published,
                authorization_revoked: true,
                ..snapshot.clone()
            },
        ] {
            assert!(CanonicalPublication::from_durable_snapshot(malformed).is_err());
        }
    }

    #[test]
    fn cross_publication_permit_cannot_authorize_at_same_epoch() {
        let mut a = CanonicalPublication::new("owner-a", "persona-a", "publication-a", 3).unwrap();
        let mut b = CanonicalPublication::new("owner-b", "persona-b", "publication-b", 3).unwrap();
        a.publish("owner-a", 3, true).unwrap();
        b.publish("owner-b", 3, true).unwrap();
        let from_a = a.visitor_lease(true).unwrap();
        assert_eq!(
            b.authorize_visitor(true, "owner-b", "persona-b", 3, &from_a, 10),
            Err(PublicationError::WrongPublication)
        );
        a.pause("owner-a").unwrap();
        assert_eq!(
            b.authorize_visitor(true, "owner-b", "persona-b", 3, &from_a, 11),
            Err(PublicationError::WrongPublication)
        );
        let from_b = b.visitor_lease(true).unwrap();
        assert_eq!(
            b.authorize_visitor(true, "owner-b", "persona-b", 3, &from_b, 12),
            Ok(())
        );
    }

    #[test]
    fn visitor_must_have_correct_canonical_owner_persona_and_identity() {
        let mut p = CanonicalPublication::new("owner", "persona", "publication-main", 2).unwrap();
        p.publish("owner", 2, true).unwrap();
        let lease = p.visitor_lease(true).unwrap();
        assert!(
            p.authorize_visitor(false, "owner", "persona", 2, &lease, 1)
                .is_err()
        );
        assert!(
            p.authorize_visitor(true, "other", "persona", 2, &lease, 1)
                .is_err()
        );
        assert!(
            p.authorize_visitor(true, "owner", "other", 2, &lease, 1)
                .is_err()
        );
        assert!(
            p.authorize_visitor(true, "owner", "persona", 3, &lease, 1)
                .is_err()
        );
        p.unpublish("owner").unwrap();
        assert!(
            p.authorize_visitor(true, "owner", "persona", 2, &lease, 1)
                .is_err()
        );
    }
}
