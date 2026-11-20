//! Single-owner pre-entry workspace operations on the SAME canonical Persona commit.
//!
//! No HTTP or public endpoint is provided by this module. The caller must
//! enforce owner authentication and reviewed readiness before publication,
//! and bind visitor turns to the server-retained visitor permit.
use parking_lot::Mutex;
use serde::Serialize;
use vpr_policy::publication_lifecycle::VisitorPermit;

use crate::persona_persistence::{PersistedPersonaV2, commit_current_record, load_current_record};
use crate::persona_transaction::{self, CANONICAL_TRANSACTION};
use crate::rt1_visitor_session_manager::{PendingVisitorRevocations, VisitorSessionManager};
use crate::rt1_workspace::OwnerWorkspaceRecord;
#[path = "rt1_workspace_accounts.rs"]
mod accounts;

#[derive(Serialize)]
pub struct WorkspaceStatus {
    pub owner_id: String,
    pub persona_id: String,
    pub persona_version: u64,
    pub publication_status: Option<&'static str>,
}

/// All RT1 workspace operations hold the shared process mutex AND the OS
/// sibling-file transaction fence across their complete read-modify-commit.
/// This serializes canonical file writers without replacing the existing
/// lifetime owner-lab lease or allowing multiple live media runtimes.
///
/// Service handle referencing the one in-process canonical workspace lock.
pub struct Rt1WorkspaceService {
    transaction: &'static Mutex<()>,
}

impl Default for Rt1WorkspaceService {
    fn default() -> Self {
        Self::new()
    }
}

impl Rt1WorkspaceService {
    #[must_use]
    pub fn new() -> Self {
        Self {
            transaction: &CANONICAL_TRANSACTION,
        }
    }

    /// Create one owner credential for an existing reviewed Persona. The
    /// raw 256-bit token is returned only after the durable commit succeeds.
    ///
    /// # Errors
    /// Refuses repeat provisioning, missing Persona and storage failures.
    pub fn provision_owner(&self) -> Result<(String, String), String> {
        let _transaction = persona_transaction::begin_workspace(self.transaction)?;
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

    /// Read the current Persona identity from the sole reviewed-owner store.
    /// No shadow registry or browser-supplied identity is trusted.
    ///
    /// # Errors
    /// Refuses missing, invalid, or unavailable reviewed Persona storage.
    pub fn current_persona_identity(&self) -> Result<(String, u64), String> {
        let _transaction = persona_transaction::begin_workspace(self.transaction)?;
        let record = required_persona()?;
        if record.snapshot.persona_id.trim().is_empty() || record.snapshot.persona_version == 0 {
            return Err("invalid canonical Persona identity".into());
        }
        Ok((record.snapshot.persona_id, record.snapshot.persona_version))
    }

    /// Select a reviewed Persona using ONLY the canonical server-issued visitor
    /// bearer. Never restore the process-global owner Persona into a visitor
    /// engine selected by another account's publication.
    pub(crate) fn reviewed_snapshot_for_visitor(
        &self,
        bearer_token: &str,
    ) -> Result<crate::owner_context::DurableReviewedOwnerContextSnapshot, String> {
        crate::persona_store_lease::require_live_runtime_owner()?;
        let _transaction = persona_transaction::begin_workspace(self.transaction)?;
        let record = required_persona()?;
        snapshot_for_visitor_in_record(&record, bearer_token, server_now_millis()?)
    }

    /// Reject a secondary tenant before any shared provider credentials are used.
    pub fn require_legacy_provider_owner(&self, bearer_token: &str) -> Result<(), String> {
        crate::persona_store_lease::require_live_runtime_owner()?;
        let _transaction = persona_transaction::begin_workspace(self.transaction)?;
        let record = required_persona()?;
        let primary = record.workspace.as_ref().ok_or("primary owner missing")?;
        admit_from_workspace(
            primary,
            bearer_token,
            &record.snapshot.persona_id,
            record.snapshot.persona_version,
            server_now_millis()?,
        )?;
        Ok(())
    }

    /// Server-side owner identification by a secret that has never been stored
    /// in plaintext. The old anonymous Owner Lab CSRF token is not an account.
    ///
    /// # Errors
    /// Denies wrong credentials or missing reviewed Persona.
    pub fn owner_status(&self, owner_token: &str) -> Result<WorkspaceStatus, String> {
        self.status_for_authenticated_owner(owner_token)
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
        let _transaction = persona_transaction::begin_workspace(self.transaction)?;
        let mut record = required_persona()?;
        let (workspace, persona_id, persona_version) =
            authenticated_owner_workspace_mut(&mut record, owner_token)?;
        if persona_version != expected_persona_version {
            return Err("stale Persona version".into());
        }
        let visitor_token = workspace.publish(
            owner_token,
            &persona_id,
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

    /// Durably pause the publication and atomically remove all previously
    /// registered local visitor transports before any concurrent republish
    /// can admit a new visitor. The caller must run the returned stops AFTER
    /// this transaction releases its lock.
    ///
    /// # Errors
    /// Refuses invalid owner, missing storage, or failed durable commit.
    pub(crate) fn pause_and_collect_revocations(
        &self,
        owner_token: &str,
        visitors: &VisitorSessionManager,
    ) -> Result<PendingVisitorRevocations, String> {
        self.change_publication_and_collect(owner_token, OwnerWorkspaceRecord::pause, visitors)
    }

    /// Same atomic commit-and-drain boundary for unpublication.
    ///
    /// # Errors
    /// Refuses invalid owner, missing storage, or failed durable commit.
    pub(crate) fn unpublish_and_collect_revocations(
        &self,
        owner_token: &str,
        visitors: &VisitorSessionManager,
    ) -> Result<PendingVisitorRevocations, String> {
        self.change_publication_and_collect(owner_token, OwnerWorkspaceRecord::unpublish, visitors)
    }

    /// Authenticate a visitor independently from owner CSRF and issue a
    /// server-retained permit for this exact publication.
    ///
    /// # Errors
    /// Refuses bad token and any non-published Persona.
    pub fn visitor_permit(&self, bearer_token: &str) -> Result<VisitorPermit, String> {
        let _transaction = persona_transaction::begin_workspace(self.transaction)?;
        let record = required_persona()?;
        if let Some(primary) = &record.workspace {
            if let Ok(permit) = primary.visitor_permit(bearer_token) {
                return Ok(permit);
            }
        }
        for additional in record.additional_owner_personas.values() {
            if let Ok(permit) = additional.workspace.visitor_permit(bearer_token) {
                return Ok(permit);
            }
        }
        Err("visitor authorization denied".into())
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
        let _transaction = persona_transaction::begin_workspace(self.transaction)?;
        let record = required_persona()?;
        admit_visitor_in_record(&record, bearer_token, now_millis)
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
        let _transaction = persona_transaction::begin_workspace(self.transaction)?;
        let record = required_persona()?;
        let (workspace, persona_id, version) = published_visitor_workspace(&record, permit)?;
        workspace.authorize_visitor(bearer_token, permit, persona_id, version, now_millis)
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

    /// Serialize one short, bounded local enqueue with owner pause/unpublish.
    /// A `SyncSender` guarantees a nonblocking local `try_send`; callers cannot
    /// substitute arbitrary provider/network side effects while holding the
    /// authorization transaction. The consumer MUST separately re-authorize
    /// every playback/egress and discard queued media after revocation.
    /// This is not an RT1 endpoint or a provider-egress permission.
    ///
    /// # Errors
    /// Refuses stale permits, unconfigured storage, full or closed queues.
    pub fn enqueue_authorized_visitor_fragment<T>(
        &self,
        bearer_token: &str,
        permit: &VisitorPermit,
        sender: &std::sync::mpsc::SyncSender<T>,
        fragment: T,
    ) -> Result<(), String> {
        let _transaction = persona_transaction::begin_workspace(self.transaction)?;
        let record = required_persona()?;
        let (workspace, persona_id, version) = published_visitor_workspace(&record, permit)?;
        enqueue_from_workspace(
            workspace,
            (bearer_token, permit),
            (persona_id, version),
            server_now_millis()?,
            sender,
            fragment,
        )
    }

    /// Atomically admit a locally tracked provider operation under the same
    /// publication transaction as owner pause/unpublish. The callback may
    /// ONLY register an in-process cancellation/drain lease; it MUST NOT
    /// perform network I/O, block on a provider or acquire the engine mutex.
    ///
    /// # Errors
    /// Denies stale publication or failed local registration.
    pub(crate) fn admit_visitor_provider_egress<T>(
        &self,
        bearer_token: &str,
        permit: &VisitorPermit,
        register: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        crate::persona_store_lease::require_live_runtime_owner()?;
        let _transaction = persona_transaction::begin_workspace(self.transaction)?;
        let record = required_persona()?;
        let (workspace, persona_id, version) = published_visitor_workspace(&record, permit)?;
        workspace.authorize_visitor(
            bearer_token,
            permit,
            persona_id,
            version,
            server_now_millis()?,
        )?;
        register()
    }

    /// Invalidate all pre-crash visitor session permits on process restart.
    /// This must durably commit BEFORE exposing an RT1 visitor listener.
    ///
    /// # Errors
    /// Fails startup on an unreadable or unwritable publication record.
    pub fn recover_before_listener(&self) -> Result<(), String> {
        let _transaction = persona_transaction::begin_workspace(self.transaction)?;
        let Some(mut record) = load_current_record()? else {
            return Ok(());
        };
        if let Some(workspace) = record.workspace.as_mut() {
            workspace.invalidate_restart_leases()?;
        }
        for additional in record.additional_owner_personas.values_mut() {
            additional.workspace.invalidate_restart_leases()?;
        }
        commit_current_record(&record)?;
        Ok(())
    }

    fn change_publication_and_collect(
        &self,
        owner_token: &str,
        operation: fn(&mut OwnerWorkspaceRecord, &str) -> Result<(), String>,
        visitors: &VisitorSessionManager,
    ) -> Result<PendingVisitorRevocations, String> {
        let _transaction = persona_transaction::begin_workspace(self.transaction)?;
        let mut record = required_persona()?;
        let (workspace, _, _) = authenticated_owner_workspace_mut(&mut record, owner_token)?;
        operation(workspace, owner_token)?;
        let revoked_owner_id = workspace.owner_id.clone();
        // Never stop a transport for a mutation that failed to commit.
        commit_current_record(&record)?;
        // Hold the same transaction while draining: a concurrent republish
        // cannot register its new-epoch visitors before this drain. Never
        // revoke sessions belonging to an unrelated owner/workspace.
        Ok(visitors.take_for_revocation_for_owner(&revoked_owner_id))
    }

    fn change_publication(
        &self,
        owner_token: &str,
        operation: fn(&mut OwnerWorkspaceRecord, &str) -> Result<(), String>,
    ) -> Result<(), String> {
        let _transaction = persona_transaction::begin_workspace(self.transaction)?;
        let mut record = required_persona()?;
        let (workspace, _, _) = authenticated_owner_workspace_mut(&mut record, owner_token)?;
        operation(workspace, owner_token)?;
        commit_current_record(&record)
    }
}

/// Validate the current publication before inserting into a bounded local
/// queue. This is not an egress fence; the consumer must check again.
fn enqueue_from_workspace<T>(
    workspace: &OwnerWorkspaceRecord,
    visitor: (&str, &VisitorPermit),
    persona: (&str, u64),
    now_millis: u64,
    sender: &std::sync::mpsc::SyncSender<T>,
    fragment: T,
) -> Result<(), String> {
    workspace.authorize_visitor(visitor.0, visitor.1, persona.0, persona.1, now_millis)?;
    sender
        .try_send(fragment)
        .map_err(|_| "visitor local queue unavailable".to_owned())
}

/// Read a fresh trusted server clock at each authorization boundary.
/// Clock failures are denials, never implicit infinite leases.
pub(crate) fn server_now_millis() -> Result<u64, String> {
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
    // Revalidate even when the computation fails: a concurrent owner pause,
    // correction or unpublish must not be hidden by a provider error.
    // Canonical revocation takes precedence over that computation error.
    let result = compute();
    authorize(clock()?.max(started_at))?;
    result
}

fn required_persona() -> Result<PersistedPersonaV2, String> {
    load_current_record()?.ok_or("reviewed Persona required".into())
}

/// Route mutation through credential-authenticated canonical owner data.
/// Never trust a browser-supplied owner identifier or default to primary.
fn authenticated_owner_workspace_mut<'a>(
    record: &'a mut PersistedPersonaV2,
    owner_token: &str,
) -> Result<(&'a mut OwnerWorkspaceRecord, String, u64), String> {
    if let Some(primary) = record.workspace.as_mut() {
        if primary.authenticate_owner(owner_token) {
            return Ok((
                primary,
                record.snapshot.persona_id.clone(),
                record.snapshot.persona_version,
            ));
        }
    }
    for additional in record.additional_owner_personas.values_mut() {
        if additional.workspace.authenticate_owner(owner_token) {
            return Ok((
                &mut additional.workspace,
                additional.snapshot.persona_id.clone(),
                additional.snapshot.persona_version,
            ));
        }
    }
    Err("owner authorization denied".into())
}

/// The visitor permit fixes tenant identity; no fallback to another tenant
/// after permit issuance, even if the original owner was paused/revoked.
fn published_visitor_workspace<'a>(
    record: &'a PersistedPersonaV2,
    permit: &VisitorPermit,
) -> Result<(&'a OwnerWorkspaceRecord, &'a str, u64), String> {
    if let Some(primary) = &record.workspace {
        if primary.owner_id == permit.owner_id() {
            return Ok((
                primary,
                &record.snapshot.persona_id,
                record.snapshot.persona_version,
            ));
        }
    }
    let owned = record
        .additional_owner_personas
        .get(permit.owner_id())
        .ok_or("visitor owner not found")?;
    Ok((
        &owned.workspace,
        &owned.snapshot.persona_id,
        owned.snapshot.persona_version,
    ))
}

fn admit_visitor_in_record(
    record: &PersistedPersonaV2,
    bearer_token: &str,
    now_millis: u64,
) -> Result<VisitorPermit, String> {
    if let Some(primary) = &record.workspace {
        if let Ok(permit) = admit_from_workspace(
            primary,
            bearer_token,
            &record.snapshot.persona_id,
            record.snapshot.persona_version,
            now_millis,
        ) {
            return Ok(permit);
        }
    }
    for additional in record.additional_owner_personas.values() {
        if let Ok(permit) = admit_from_workspace(
            &additional.workspace,
            bearer_token,
            &additional.snapshot.persona_id,
            additional.snapshot.persona_version,
            now_millis,
        ) {
            return Ok(permit);
        }
    }
    Err("visitor authorization denied".into())
}

/// A pure lookup against an already validated canonical envelope. Denies
/// unknown, expired, paused or stale visitor tokens before selecting a Persona.
fn snapshot_for_visitor_in_record(
    record: &PersistedPersonaV2,
    bearer_token: &str,
    now_millis: u64,
) -> Result<crate::owner_context::DurableReviewedOwnerContextSnapshot, String> {
    if let Some(workspace) = &record.workspace {
        if admit_from_workspace(
            workspace,
            bearer_token,
            &record.snapshot.persona_id,
            record.snapshot.persona_version,
            now_millis,
        )
        .is_ok()
        {
            return Ok(record.snapshot.clone());
        }
    }
    for additional in record.additional_owner_personas.values() {
        if admit_from_workspace(
            &additional.workspace,
            bearer_token,
            &additional.snapshot.persona_id,
            additional.snapshot.persona_version,
            now_millis,
        )
        .is_ok()
        {
            return Ok(additional.snapshot.clone());
        }
    }
    Err("visitor authorization denied".into())
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
#[path = "rt1_workspace_service_tests.rs"]
mod tests;
