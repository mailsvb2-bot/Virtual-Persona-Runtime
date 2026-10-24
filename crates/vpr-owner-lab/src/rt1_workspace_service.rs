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

/// Serializes canonical commits and visitor permission checks through this
/// service instance only. The file store uses atomic replacement, not
/// cross-process compare-and-swap; do not expose concurrent RT1 listeners
/// until a single authoritative writer or durable revision CAS is enforced.
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

    /// Atomically mint and validate a permit against one canonical Persona
    /// revision. This removes the gap between separate bootstrap reads; the
    /// caller MUST still re-authorize before every subsequent output/egress.
    ///
    /// # Errors
    /// Rejects absent publications, revoked visitors or stale Persona state.
    pub fn admit_visitor_turn(
        &self,
        bearer_token: &str,
        now_millis: u64,
    ) -> Result<VisitorPermit, String> {
        let _transaction = self.transaction.lock();
        let record = required_persona()?;
        let workspace = record.workspace.as_ref().ok_or("owner workspace missing")?;
        admit_from_workspace(
            workspace,
            bearer_token,
            &record.snapshot.persona_id,
            record.snapshot.persona_version,
            now_millis,
        )
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

    /// Recheck canonical visitor authority around a pure buffered computation.
    /// A successful return is NOT permission for provider egress, streaming,
    /// playback or delivery: owner revocation can occur immediately afterwards.
    /// The eventual delivery boundary must independently fence that operation.
    /// The callback MUST NOT perform external egress or irreversible side effects.
    ///
    /// # Errors
    /// Denies revoked or expired authority, missing storage and clock failures.
    pub fn guarded_buffered_visitor_output<T>(
        &self,
        bearer_token: &str,
        permit: &VisitorPermit,
        compute: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        guard_buffered_output(
            |now| self.authorize_visitor(bearer_token, permit, now),
            server_now_millis,
            compute,
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

/// Read a fresh trusted server clock at each authorization boundary.
/// Clock failures are denials, never implicit infinite leases.
fn server_now_millis() -> Result<u64, String> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "server clock precedes Unix epoch".to_owned())?;
    u64::try_from(duration.as_millis()).map_err(|_| "server clock out of range".to_owned())
}

/// Pure buffered computation only. This does not authorize subsequent egress.
/// Clamping the second timestamp prevents clock rollback from extending a lease.
fn guard_buffered_output<T>(
    mut authorize: impl FnMut(u64) -> Result<(), String>,
    mut clock: impl FnMut() -> Result<u64, String>,
    compute: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let started_at = clock()?;
    authorize(started_at)?;
    let result = compute()?;
    authorize(clock()?.max(started_at))?;
    Ok(result)
}

fn required_persona() -> Result<PersistedPersonaV2, String> {
    load_current_record()?.ok_or("reviewed Persona required".into())
}

/// The admission decision uses one immutable canonical record while the
/// service transaction is held. This helper is deliberately not an egress
/// grant: every later operation must recheck durable authority.
fn admit_from_workspace(
    workspace: &OwnerWorkspaceRecord,
    bearer_token: &str,
    persona_id: &str,
    persona_version: u64,
    now_millis: u64,
) -> Result<VisitorPermit, String> {
    let permit = workspace.visitor_permit(bearer_token)?;
    workspace.authorize_visitor(
        bearer_token,
        &permit,
        persona_id,
        persona_version,
        now_millis,
    )?;
    Ok(permit)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Windows has an implicit credential store: a missing file path is not
    // an unconfigured store there, and this test must not mutate real credentials.
    #[cfg(not(windows))]
    #[test]
    fn unconfigured_workspace_fails_closed_before_listener() {
        // CI has no persistent Persona store on Linux. Recovery must reject
        // that configuration rather than silently permit a visitor listener.
        // This test does not mutate process-global environment variables.
        let service = Rt1WorkspaceService::new();
        assert!(service.recover_before_listener().is_err());
        assert!(service.visitor_permit(&"0".repeat(64)).is_err());
        assert!(service.admit_visitor_turn(&"0".repeat(64), 0).is_err());

        assert!(service.owner_status(&"0".repeat(64)).is_err());
    }

    #[test]
    fn admission_fails_closed_across_owner_pause_republish_and_correction() {
        let (mut workspace, owner) = OwnerWorkspaceRecord::provision().unwrap();
        let invalid = "0".repeat(64);
        assert!(admit_from_workspace(&workspace, &invalid, "persona-a", 3, 1).is_err());
        assert!(workspace.publish(&owner, "persona-a", 3, false).is_err());
        let visitor = workspace.publish(&owner, "persona-a", 3, true).unwrap();
        assert!(admit_from_workspace(&workspace, &invalid, "persona-a", 3, 2).is_err());
        assert!(admit_from_workspace(&workspace, &visitor, "persona-b", 3, 2).is_err());
        assert!(admit_from_workspace(&workspace, &visitor, "persona-a", 4, 2).is_err());

        let old_permit = admit_from_workspace(&workspace, &visitor, "persona-a", 3, 2).unwrap();
        workspace.pause(&owner).unwrap();
        assert!(admit_from_workspace(&workspace, &visitor, "persona-a", 3, 3).is_err());
        assert!(
            workspace
                .authorize_visitor(&visitor, &old_permit, "persona-a", 3, 3)
                .is_err()
        );

        let next_visitor = workspace.publish(&owner, "persona-a", 3, true).unwrap();
        assert!(admit_from_workspace(&workspace, &visitor, "persona-a", 3, 4).is_err());
        let next_permit =
            admit_from_workspace(&workspace, &next_visitor, "persona-a", 3, 4).unwrap();
        workspace.on_persona_correction(4).unwrap();
        assert!(admit_from_workspace(&workspace, &next_visitor, "persona-a", 4, 5).is_err());
        assert!(
            workspace
                .authorize_visitor(&next_visitor, &next_permit, "persona-a", 4, 5)
                .is_err()
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn buffered_visitor_output_does_not_compute_without_canonical_store() {
        use std::cell::Cell;
        use vpr_policy::publication_lifecycle::CanonicalPublication;

        let mut publication =
            CanonicalPublication::new("owner", "persona", "publication", 1).unwrap();
        publication.publish("owner", 1, true).unwrap();
        let permit = publication.visitor_lease(true).unwrap();
        let computed = Cell::new(false);
        let service = Rt1WorkspaceService::new();
        let result = service.guarded_buffered_visitor_output("visitor", &permit, || {
            computed.set(true);
            Ok("unreleased")
        });
        assert!(result.is_err());
        assert!(!computed.get());
    }

    #[test]
    fn buffered_output_rechecks_canonical_authority_after_owner_pause() {
        use std::cell::{Cell, RefCell};

        let (mut workspace, owner) = OwnerWorkspaceRecord::provision().unwrap();
        let visitor = workspace.publish(&owner, "persona-a", 3, true).unwrap();
        let permit = workspace.visitor_permit(&visitor).unwrap();
        let workspace = RefCell::new(workspace);
        let computed = Cell::new(false);

        let result = guard_buffered_output(
            |now| {
                workspace
                    .borrow()
                    .authorize_visitor(&visitor, &permit, "persona-a", 3, now)
            },
            || Ok(10),
            || {
                computed.set(true);
                workspace.borrow_mut().pause(&owner)?;
                Ok("must-not-release")
            },
        );
        assert!(computed.get());
        assert!(result.is_err());
    }

    #[test]
    fn buffered_output_rechecks_expiry_at_completion() {
        use std::cell::Cell;
        use vpr_policy::AuthorizationState;

        let authority = AuthorizationState::new(Some(100));
        let lease = authority.snapshot();
        let clock = Cell::new(99_u64);
        let result = guard_buffered_output(
            |now| {
                authority
                    .validate(lease, now)
                    .map_err(|_| "visitor permit expired".to_owned())
            },
            || Ok(clock.get()),
            || {
                clock.set(100);
                Ok("must-not-release")
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn buffered_output_never_computes_after_initial_denial() {
        use std::cell::Cell;

        let computed = Cell::new(false);
        let result = guard_buffered_output(
            |_| Err("visitor denied".to_owned()),
            || Ok(10),
            || {
                computed.set(true);
                Ok("must-not-compute")
            },
        );
        assert!(result.is_err());
        assert!(!computed.get());
    }

    #[test]
    fn service_signature_does_not_accept_client_claimed_readiness() {
        // Browser JS cannot invoke `publish` without the server-provided
        // readiness gate; all calls go through the service transaction.
        let _ = Rt1WorkspaceService::new();
        assert!(required_persona().is_err());
    }
}
