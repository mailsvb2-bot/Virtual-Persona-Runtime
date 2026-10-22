//! Owner identity and publication metadata for the canonical Persona envelope.
//! No standalone workspace file, public listener, or second policy authority.
//! Public access remains disabled until the RT0 exact exit evidence is accepted.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use vpr_policy::publication_lifecycle::{
    CanonicalPublication, CanonicalPublicationSnapshot, PublicationError, PublicationStatus,
    VisitorPermit,
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct OwnerWorkspaceRecord {
    pub owner_id: String,
    owner_token_sha256: String,
    publication: Option<CanonicalPublicationSnapshot>,
    visitor_token_sha256: Option<String>,
}

impl OwnerWorkspaceRecord {
    /// Provision the initial owner identity using cryptographic random tokens.
    /// Token plaintext is returned ONCE and never saved in the Persona record.
    pub fn provision() -> Result<(Self, String), String> {
        let owner_id = generate_token(16)?;
        let token = generate_token(32)?;
        Ok((
            Self {
                owner_id,
                owner_token_sha256: hash_secret(&token),
                publication: None,
                visitor_token_sha256: None,
            },
            token,
        ))
    }

    #[must_use]
    pub fn authenticate_owner(&self, token: &str) -> bool {
        token_matches(&self.owner_token_sha256, token)
    }

    /// Establish a version-pinned publication after the real server has
    /// independently verified consent, readiness and the cost gate.
    pub fn publish(
        &mut self,
        owner_token: &str,
        persona_id: &str,
        persona_version: u64,
        readiness_authorized: bool,
    ) -> Result<String, String> {
        if !self.authenticate_owner(owner_token) {
            return Err("owner authorization denied".into());
        }
        let mut publication = match &self.publication {
            Some(state) => CanonicalPublication::from_durable_snapshot(state.clone())
                .map_err(|_| "invalid canonical publication")?,
            None => CanonicalPublication::new(
                &self.owner_id,
                persona_id,
                &generate_token(16)?,
                persona_version,
            )
            .map_err(|_| "invalid canonical publication")?,
        };
        if self
            .publication
            .as_ref()
            .is_some_and(|state| state.persona_id != persona_id)
        {
            return Err("publication Persona mismatch".into());
        }
        publication
            .publish(&self.owner_id, persona_version, readiness_authorized)
            .map_err(|_| "publication denied")?;
        let visitor_token = generate_token(32)?;
        self.visitor_token_sha256 = Some(hash_secret(&visitor_token));
        self.publication = Some(publication.durable_snapshot());
        Ok(visitor_token)
    }

    /// Revoke both the current publication epoch and its shareable visitor token.
    pub fn pause(&mut self, owner_token: &str) -> Result<(), String> {
        self.change_publication(owner_token, CanonicalPublication::pause)
    }

    /// Unpublish without deleting private Persona history.
    pub fn unpublish(&mut self, owner_token: &str) -> Result<(), String> {
        self.change_publication(owner_token, CanonicalPublication::unpublish)
    }

    /// Invalidates stale derived/public state in the SAME durable commit that
    /// advances the canonical reviewed Persona revision.
    pub fn on_persona_correction(&mut self, new_persona_version: u64) -> Result<(), String> {
        if let Some(snapshot) = self.publication.as_ref() {
            let mut publication = CanonicalPublication::from_durable_snapshot(snapshot.clone())
                .map_err(|_| "invalid canonical publication")?;
            publication
                .correct_persona(&self.owner_id, new_persona_version)
                .map_err(|_| "publication correction failed")?;
            self.publication = Some(publication.durable_snapshot());
            self.visitor_token_sha256 = None;
        }
        Ok(())
    }

    /// Reject expired process/session permits after a server restart. The new
    /// snapshot MUST be committed before opening the owner/visitor listeners.
    pub fn invalidate_restart_leases(&mut self) -> Result<(), String> {
        if let Some(snapshot) = self.publication.as_ref() {
            let mut publication = CanonicalPublication::from_durable_snapshot(snapshot.clone())
                .map_err(|_| "invalid canonical publication")?;
            publication
                .invalidate_restart_leases()
                .map_err(|_| "publication epoch exhausted")?;
            self.publication = Some(publication.durable_snapshot());
        }
        Ok(())
    }

    #[must_use]
    pub fn status(&self) -> Option<PublicationStatus> {
        self.publication.as_ref().map(|state| state.status)
    }

    /// Authenticate the bearer token independently of an owner browser
    /// session. Returns a server-side visitor permit, not a user-supplied epoch.
    pub fn visitor_permit(&self, token: &str) -> Result<VisitorPermit, String> {
        let stored = self
            .visitor_token_sha256
            .as_deref()
            .ok_or("visitor access unavailable")?;
        if !token_matches(stored, token) {
            return Err("visitor authorization denied".into());
        }
        let state = self.publication.as_ref().ok_or("publication missing")?;
        let publication = CanonicalPublication::from_durable_snapshot(state.clone())
            .map_err(|_| "invalid canonical publication")?;
        publication
            .visitor_lease(true)
            .map_err(|_| "visitor access denied".into())
    }

    /// Recheck current durable publication and Persona version immediately
    /// before provider egress, never only during the initial visitor bootstrap.
    pub fn authorize_visitor(
        &self,
        token: &str,
        permit: &VisitorPermit,
        current_persona: &str,
        current_persona_version: u64,
        now_millis: u64,
    ) -> Result<(), String> {
        let stored = self
            .visitor_token_sha256
            .as_deref()
            .ok_or("visitor access unavailable")?;
        if !token_matches(stored, token) {
            return Err("visitor authorization denied".into());
        }
        let state = self.publication.as_ref().ok_or("publication missing")?;
        let publication = CanonicalPublication::from_durable_snapshot(state.clone())
            .map_err(|_| "invalid canonical publication")?;
        publication
            .authorize_visitor(
                true,
                &self.owner_id,
                current_persona,
                current_persona_version,
                permit,
                now_millis,
            )
            .map_err(|_| "visitor access denied".into())
    }

    /// Validate the loaded record against its sole canonical Persona.
    pub fn validate(&self, persona_id: &str, persona_version: u64) -> Result<(), String> {
        if self.owner_id.len() != 32 || !valid_sha256(&self.owner_token_sha256) {
            return Err("invalid owner identity".into());
        }
        if let Some(publication) = &self.publication {
            if publication.owner_id != self.owner_id
                || publication.persona_id != persona_id
                || publication.persona_version > persona_version
            {
                return Err("publication does not match canonical Persona".into());
            }
            CanonicalPublication::from_durable_snapshot(publication.clone())
                .map_err(|_| "invalid publication snapshot")?;
            if publication.status == PublicationStatus::Published
                && publication.persona_version != persona_version
            {
                return Err("stale publication must be revoked".into());
            }
        }
        if let Some(hash) = &self.visitor_token_sha256 {
            if !valid_sha256(hash)
                || self
                    .publication
                    .as_ref()
                    .is_none_or(|p| p.status != PublicationStatus::Published)
            {
                return Err("invalid visitor authority state".into());
            }
        }
        Ok(())
    }

    fn change_publication(
        &mut self,
        owner_token: &str,
        operation: fn(&mut CanonicalPublication, &str) -> Result<(), PublicationError>,
    ) -> Result<(), String> {
        if !self.authenticate_owner(owner_token) {
            return Err("owner authorization denied".into());
        }
        let state = self.publication.as_ref().ok_or("publication missing")?;
        let mut publication = CanonicalPublication::from_durable_snapshot(state.clone())
            .map_err(|_| "invalid canonical publication")?;
        operation(&mut publication, &self.owner_id).map_err(|_| "publication mutation denied")?;
        self.publication = Some(publication.durable_snapshot());
        self.visitor_token_sha256 = None;
        Ok(())
    }
}

fn generate_token(bytes: usize) -> Result<String, String> {
    let mut random = vec![0_u8; bytes];
    getrandom::fill(&mut random).map_err(|_| "secure random generator unavailable")?;
    let mut hex = String::with_capacity(bytes * 2);
    for byte in random {
        write!(&mut hex, "{byte:02x}").map_err(|_| "token encoding failed")?;
    }
    Ok(hex)
}

fn hash_secret(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest {
        write!(&mut hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    hex
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn token_matches(stored_hash: &str, candidate: &str) -> bool {
    if !valid_sha256(stored_hash) || candidate.len() != 64 {
        return false;
    }
    let actual = hash_secret(candidate);
    let mut difference = 0_u8;
    for (expected, received) in stored_hash.bytes().zip(actual.bytes()) {
        difference |= expected ^ received;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_distinct_private_and_visitor_denied_after_pause() {
        let (mut workspace, owner_token) = OwnerWorkspaceRecord::provision().unwrap();
        assert!(workspace.authenticate_owner(&owner_token));
        assert!(!workspace.authenticate_owner(&"0".repeat(64)));
        let visitor = workspace
            .publish(&owner_token, "persona-a", 3, true)
            .unwrap();
        assert_ne!(owner_token, visitor);
        let permit = workspace.visitor_permit(&visitor).unwrap();
        workspace
            .authorize_visitor(&visitor, &permit, "persona-a", 3, 10)
            .unwrap();
        assert!(
            workspace
                .authorize_visitor(&visitor, &permit, "persona-a", 4, 10)
                .is_err()
        );
        workspace.pause(&owner_token).unwrap();
        assert!(workspace.visitor_permit(&visitor).is_err());
        assert!(
            workspace
                .authorize_visitor(&visitor, &permit, "persona-a", 3, 11)
                .is_err()
        );
        assert_eq!(workspace.status(), Some(PublicationStatus::Paused));
    }

    #[test]
    fn correction_and_restart_never_restore_prior_session_authority() {
        let (mut workspace, owner_token) = OwnerWorkspaceRecord::provision().unwrap();
        let visitor = workspace
            .publish(&owner_token, "persona-a", 7, true)
            .unwrap();
        let old_permit = workspace.visitor_permit(&visitor).unwrap();
        let raw = serde_json::to_string(&workspace).unwrap();
        let mut restored: OwnerWorkspaceRecord = serde_json::from_str(&raw).unwrap();
        restored.invalidate_restart_leases().unwrap();
        assert!(
            restored
                .authorize_visitor(&visitor, &old_permit, "persona-a", 7, 20)
                .is_err()
        );
        let current_permit = restored.visitor_permit(&visitor).unwrap();
        restored
            .authorize_visitor(&visitor, &current_permit, "persona-a", 7, 21)
            .unwrap();
        restored.on_persona_correction(8).unwrap();
        assert_eq!(restored.status(), Some(PublicationStatus::Unpublished));
        assert!(restored.visitor_permit(&visitor).is_err());
        assert!(
            restored
                .publish(&owner_token, "persona-a", 7, true)
                .is_err()
        );
        let renewed = restored
            .publish(&owner_token, "persona-a", 8, true)
            .unwrap();
        assert_ne!(renewed, visitor);
        assert!(restored.visitor_permit(&visitor).is_err());
        assert!(restored.visitor_permit(&renewed).is_ok());
    }
}
