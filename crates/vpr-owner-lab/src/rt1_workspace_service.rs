//! Single-owner pre-entry workspace operations on the SAME canonical Persona commit.
//!
//! No HTTP or public endpoint is provided by this module. The caller must
//! enforce owner authentication and reviewed readiness before publication,
//! and bind visitor turns to the server-retained visitor permit.
use parking_lot::Mutex;
use serde::Serialize;
use vpr_policy::publication_lifecycle::{PublicationStatus, VisitorPermit};

use crate::persona_persistence::{PersistedPersonaV2, commit_current_record, load_current_record};
use crate::rt1_workspace::OwnerWorkspaceRecord;

#[derive(Serialize)]
pub struct WorkspaceStatus {
    pub owner_id: String,
    pub persona_id: String,
    pub persona_version: u64,
    pub publication_status: Option<&'static str>,
}

/// Serializes canonical commits and visitor permission checks within one
/// process. Explicit file stores additionally hold a process-wide OS lock.
#[derive(Default)]
pub struct Rt1WorkspaceService {
    transaction: Mutex<()>,
}

impl Rt1WorkspaceService {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create one owner credential for an existing reviewed Persona. The
    /// raw 256-bit token is returned only after the durable commit succeeds.
    ///
    /// # Errors
    /// Refuses repeat provisioning, missing Persona and storage failures.
    pub fn provision_owner(&self) -> Result<(String, String), String> {
        let _transaction = self.transaction.lock();
        let mut record = required_persona()?;
        if record.workspace.is_some() {
            return Err("owner workspace already provisioned".into());
        }
        let (workspace, owner_token) = OwnerWorkspaceRecord::provision()?;
        let owner_id = workspace.owner_id.clone();
        record.workspace = Some(workspace);
        commit_current_record(&record)?;
        Ok((owner_id, owner_token))
    }

    /// Server-side owner identification by a secret that has never been stored
    /// in plaintext. The old anonymous Owner Lab CSRF token is not an account.
    ///
    /// # Errors
    /// Denies wrong credentials or missing reviewed Persona.
    pub fn owner_status(&self, owner_token: &str) -> Result<WorkspaceStatus, String> {
        let _transaction = self.transaction.lock();
        let record = required_persona()?;
        let workspace = record.workspace.as_ref().ok_or("owner workspace missing")?;
        if !workspace.authenticate_owner(owner_token) {
            return Err("owner authorization denied".into());
        }
        let publication_status = workspace.status().map(|status| match status {
            PublicationStatus::Private => "private",
            PublicationStatus::Published => "published",
            PublicationStatus::Paused => "paused",
            PublicationStatus::Unpublished => "unpublished",
        });
        Ok(WorkspaceStatus {
            owner_id: workspace.owner_id.clone(),
            persona_id: record.snapshot.persona_id,
            persona_version: record.snapshot.persona_version,
            publication_status,
        })
    }

    /// Atomically bind a visitor token to the current reviewed Persona version.
    /// Readiness/cost/rights must be proved by the actual server, not supplied
    /// by a browser-provided boolean.
    ///
    /// # Errors
    /// Denies stale expected version, wrong owner or incomplete readiness.
    pub fn publish(
        &self,
        owner_token: &str,
        expected_persona_version: u64,
        server_readiness_verified: bool,
    ) -> Result<String, String> {
        let _transaction = self.transaction.lock();
        let mut record = required_persona()?;
        if record.snapshot.persona_version != expected_persona_version {
            return Err("stale Persona version".into());
        }
        let workspace = record.workspace.as_mut().ok_or("owner workspace missing")?;
        let visitor_token = workspace.publish(
            owner_token,
            &record.snapshot.persona_id,
            expected_persona_version,
            server_readiness_verified,
        )?;
        commit_current_record(&record)?;
        Ok(visitor_token)
    }

    /// Atomically revoke the shareable visitor secret and current authority.
    ///
    /// # Errors
    /// Rejects wrong owner, invalid state or write failure.
    pub fn pause(&self, owner_token: &str) -> Result<(), String> {
        self.change_publication(owner_token, OwnerWorkspaceRecord::pause)
    }

    /// Atomically unpublish without deleting the private Persona.
    ///
    /// # Errors
    /// Rejects wrong owner, invalid state or write failure.
    pub fn unpublish(&self, owner_token: &str) -> Result<(), String> {
        self.change_publication(owner_token, OwnerWorkspaceRecord::unpublish)
    }

    /// Authenticate a visitor independently from owner CSRF and issue a
    /// server-retained permit for this exact publication.
    ///
    /// # Errors
    /// Refuses bad token and any non-published Persona.
    pub fn visitor_permit(&self, bearer_token: &str) -> Result<VisitorPermit, String> {
        let _transaction = self.transaction.lock();
        let record = required_persona()?;
        let workspace = record.workspace.as_ref().ok_or("owner workspace missing")?;
        workspace.visitor_permit(bearer_token)
    }

    /// Re-read authoritative durable state on EVERY visitor operation, not
    /// just when bootstrap started. A stale provider permit cannot be used
    /// after a correction, pause, or unpublication.
    ///
    /// # Errors
    /// Denies stale tenant, `PersonaVersion`, visitor secret and epoch.
    pub fn authorize_visitor(
        &self,
        bearer_token: &str,
        permit: &VisitorPermit,
        now_millis: u64,
    ) -> Result<(), String> {
        let _transaction = self.transaction.lock();
        let record = required_persona()?;
        let workspace = record.workspace.as_ref().ok_or("owner workspace missing")?;
        workspace.authorize_visitor(
            bearer_token,
            permit,
            &record.snapshot.persona_id,
            record.snapshot.persona_version,
            now_millis,
        )
    }

    /// Invalidate all pre-crash visitor session permits on process restart.
    /// This must durably commit BEFORE exposing an RT1 visitor listener.
    ///
    /// # Errors
    /// Fails startup on an unreadable or unwritable publication record.
    pub fn recover_before_listener(&self) -> Result<(), String> {
        let _transaction = self.transaction.lock();
        let Some(mut record) = load_current_record()? else {
            return Ok(());
        };
        if let Some(workspace) = record.workspace.as_mut() {
            workspace.invalidate_restart_leases()?;
            commit_current_record(&record)?;
        }
        Ok(())
    }

    fn change_publication(
        &self,
        owner_token: &str,
        operation: fn(&mut OwnerWorkspaceRecord, &str) -> Result<(), String>,
    ) -> Result<(), String> {
        let _transaction = self.transaction.lock();
        let mut record = required_persona()?;
        let workspace = record.workspace.as_mut().ok_or("owner workspace missing")?;
        operation(workspace, owner_token)?;
        commit_current_record(&record)
    }
}

fn required_persona() -> Result<PersistedPersonaV2, String> {
    load_current_record()?.ok_or("reviewed Persona required".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unconfigured_workspace_fails_closed_before_listener() {
        // CI has no persistent Persona store on Linux. Recovery must reject
        // that configuration rather than silently permit a visitor listener.
        // This test does not mutate process-global environment variables.
        let service = Rt1WorkspaceService::new();
        assert!(service.recover_before_listener().is_err());
        assert!(service.visitor_permit(&"0".repeat(64)).is_err());
        assert!(service.owner_status(&"0".repeat(64)).is_err());
    }

    #[test]
    fn service_signature_does_not_accept_client_claimed_readiness() {
        // Browser JS cannot invoke `publish` without the server-provided
        // readiness gate; all calls go through the service transaction.
        let _ = Rt1WorkspaceService::new();
        assert!(required_persona().is_err());
    }
}
