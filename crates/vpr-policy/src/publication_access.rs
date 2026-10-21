//! Bounded RT1 publication-authorization feasibility contract.
//!
//! This is a pure guard over inputs from the future canonical owner workspace
//! and publication repository. It creates NO durable publication authority,
//! public route, link, or visitor token. RT1 stays blocked by its `ReleaseSpec`.
//! Before any external provider operation a future public entrypoint must
//! re-evaluate the current `Persona` version, publication state and auth epoch.

use crate::{AuthorizationSnapshot, AuthorizationState, AuthorizationValidityError};

/// One access request, resolved and authenticated by the owning repository.
/// A URL's claimed Persona or owner identity must never be trusted as this input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicationAccess<'a> {
    /// Server-resolved tenant identity for the publication owner.
    pub publication_owner: &'a str,
    /// Server-resolved current owner of the canonical Persona.
    pub persona_owner: &'a str,
    /// Server-resolved Persona identity associated with the publication.
    pub published_persona: &'a str,
    /// Server-resolved canonical Persona identity.
    pub current_persona: &'a str,
    pub published_persona_version: u64,
    pub current_persona_version: u64,
    /// True only while the canonical publication state is PUBLISHED.
    pub published: bool,
    /// Set only after visitor-specific authentication/authorization.
    pub visitor_authorized: bool,
    /// An epoch snapshot issued for the exact publication at session bootstrap.
    pub lease: AuthorizationSnapshot,
    /// Current server time; never derived from an untrusted browser clock.
    pub now_millis: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationAccessDenial {
    Unauthenticated,
    OwnerMismatch,
    PersonaMismatch,
    NotPublished,
    StalePersonaVersion,
    StaleAuthorization,
    Revoked,
    Expired,
}

/// Fail-closed permission check, not an issuance or publication service.
///
/// This must be called at BOTH visitor bootstrap and before each provider
/// operation. Its publication inputs must be loaded from a canonical,
/// transactionally consistent server-side state; a saved boolean or browser
/// status is not sufficient.
///
/// # Errors
/// Denies missing/mismatched owners, non-publication, stale Persona revisions,
/// stale sessions, revocation, and expiry.
pub fn authorize_public_visitor(
    access: PublicationAccess<'_>,
    authority: &AuthorizationState,
) -> Result<(), PublicationAccessDenial> {
    if !access.visitor_authorized {
        return Err(PublicationAccessDenial::Unauthenticated);
    }
    if access.publication_owner.is_empty()
        || access.persona_owner.is_empty()
        || access.publication_owner != access.persona_owner
    {
        return Err(PublicationAccessDenial::OwnerMismatch);
    }
    if access.published_persona.is_empty()
        || access.current_persona.is_empty()
        || access.published_persona != access.current_persona
    {
        return Err(PublicationAccessDenial::PersonaMismatch);
    }
    if !access.published {
        return Err(PublicationAccessDenial::NotPublished);
    }
    if access.published_persona_version == 0
        || access.published_persona_version != access.current_persona_version
    {
        return Err(PublicationAccessDenial::StalePersonaVersion);
    }
    match authority.validate(access.lease, access.now_millis) {
        Ok(()) => Ok(()),
        Err(AuthorizationValidityError::StaleEpoch) => {
            Err(PublicationAccessDenial::StaleAuthorization)
        }
        Err(AuthorizationValidityError::Revoked) => Err(PublicationAccessDenial::Revoked),
        Err(AuthorizationValidityError::Expired) => Err(PublicationAccessDenial::Expired),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture<'a>(lease: AuthorizationSnapshot) -> PublicationAccess<'a> {
        PublicationAccess {
            publication_owner: "tenant-a",
            persona_owner: "tenant-a",
            published_persona: "persona-1",
            current_persona: "persona-1",
            published_persona_version: 7,
            current_persona_version: 7,
            published: true,
            visitor_authorized: true,
            lease,
            now_millis: 12,
        }
    }

    #[test]
    fn valid_published_current_persona_allows_only_authorized_visitor() {
        let auth = AuthorizationState::new(Some(100));
        let access = fixture(auth.snapshot());
        assert_eq!(authorize_public_visitor(access, &auth), Ok(()));
        assert_eq!(
            authorize_public_visitor(
                PublicationAccess {
                    visitor_authorized: false,
                    ..access
                },
                &auth
            ),
            Err(PublicationAccessDenial::Unauthenticated)
        );
    }

    #[test]
    fn never_cross_tenant_or_persona_identity() {
        let auth = AuthorizationState::new(None);
        let access = fixture(auth.snapshot());
        for owner in ["", "tenant-b"] {
            assert_eq!(
                authorize_public_visitor(
                    PublicationAccess {
                        persona_owner: owner,
                        ..access
                    },
                    &auth
                ),
                Err(PublicationAccessDenial::OwnerMismatch)
            );
        }
        for persona in ["", "persona-2"] {
            assert_eq!(
                authorize_public_visitor(
                    PublicationAccess {
                        current_persona: persona,
                        ..access
                    },
                    &auth
                ),
                Err(PublicationAccessDenial::PersonaMismatch)
            );
        }
    }

    #[test]
    fn deny_paused_unpublished_or_corrected_persona_before_provider_call() {
        let auth = AuthorizationState::new(None);
        let access = fixture(auth.snapshot());
        assert_eq!(
            authorize_public_visitor(
                PublicationAccess {
                    published: false,
                    ..access
                },
                &auth
            ),
            Err(PublicationAccessDenial::NotPublished)
        );
        for version in [0, 6, 8] {
            assert_eq!(
                authorize_public_visitor(
                    PublicationAccess {
                        current_persona_version: version,
                        ..access
                    },
                    &auth
                ),
                Err(PublicationAccessDenial::StalePersonaVersion)
            );
        }
    }

    #[test]
    fn revoke_and_republish_do_not_resurrect_old_visitor_leases() {
        let mut auth = AuthorizationState::new(None);
        let old = fixture(auth.snapshot());
        auth.revoke().unwrap();
        assert_eq!(
            authorize_public_visitor(old, &auth),
            Err(PublicationAccessDenial::StaleAuthorization)
        );
        auth.replace(None).unwrap();
        assert_eq!(
            authorize_public_visitor(old, &auth),
            Err(PublicationAccessDenial::StaleAuthorization)
        );
        assert_eq!(
            authorize_public_visitor(fixture(auth.snapshot()), &auth),
            Ok(())
        );
    }

    #[test]
    fn visitor_lease_expires_at_exact_boundary() {
        let auth = AuthorizationState::new(Some(100));
        let access = fixture(auth.snapshot());
        assert_eq!(
            authorize_public_visitor(
                PublicationAccess {
                    now_millis: 99,
                    ..access
                },
                &auth
            ),
            Ok(())
        );
        assert_eq!(
            authorize_public_visitor(
                PublicationAccess {
                    now_millis: 100,
                    ..access
                },
                &auth
            ),
            Err(PublicationAccessDenial::Expired)
        );
    }
}
