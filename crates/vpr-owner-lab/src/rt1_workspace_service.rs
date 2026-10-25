//! Single-owner pre-entry workspace operations on the SAME canonical Persona commit.
//!
//! No HTTP or public endpoint is provided by this module. The caller must
//! enforce owner authentication and reviewed readiness before publication,
//! and bind visitor turns to the server-retained visitor permit.
use parking_lot::Mutex;
use serde::Serialize;
use vpr_policy::publication_lifecycle::{PublicationStatus, VisitorPermit};

use crate::persona_persistence::{PersistedPersonaV2, commit_current_record, load_current_record};
use crate::rt1_visitor_session_manager::{PendingVisitorRevocations, VisitorSessionManager};
use crate::rt1_workspace::OwnerWorkspaceRecord;

#[derive(Serialize)]
pub struct WorkspaceStatus {
    pub owner_id: String,
    pub persona_id: String,
    pub persona_version: u64,
    pub publication_status: Option<&'static str>,
}

/// Serializes all RT1 workspace operations within this process, including
/// operations issued from independent service instances. File replacement is
/// not a cross-process compare-and-swap: multiple RT1 writer processes remain
/// unsupported until durable cross-process serialization is provided.
static WORKSPACE_TRANSACTION: Mutex<()> = Mutex::new(());

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
            transaction: &WORKSPACE_TRANSACTION,
        }
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

    /// Serialize final provider dispatch with the canonical owner publication
    /// transaction. The callback MUST be bounded and MUST NOT re-enter the
    /// workspace; this cannot be used for an unbounded stream or media sender.
    /// Owner pause/unpublish cannot commit while a dispatch is in progress.
    ///
    /// # Errors
    /// Denies stale authorization before invoking the provider operation.
    pub(crate) fn guarded_provider_dispatch<T>(
        &self,
        bearer_token: &str,
        permit: &VisitorPermit,
        dispatch: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        guarded_dispatch_under_transaction(
            self.transaction,
            || {
                let record = required_persona()?;
                let workspace = record.workspace.as_ref().ok_or("owner workspace missing")?;
                workspace.authorize_visitor(
                    bearer_token,
                    permit,
                    &record.snapshot.persona_id,
                    record.snapshot.persona_version,
                    server_now_millis()?,
                )
            },
            dispatch,
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
        let _transaction = self.transaction.lock();
        let record = required_persona()?;
        let workspace = record.workspace.as_ref().ok_or("owner workspace missing")?;
        enqueue_from_workspace(
            workspace,
            (bearer_token, permit),
            (&record.snapshot.persona_id, record.snapshot.persona_version),
            server_now_millis()?,
            sender,
            fragment,
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

    fn change_publication_and_collect(
        &self,
        owner_token: &str,
        operation: fn(&mut OwnerWorkspaceRecord, &str) -> Result<(), String>,
        visitors: &VisitorSessionManager,
    ) -> Result<PendingVisitorRevocations, String> {
        let _transaction = self.transaction.lock();
        let mut record = required_persona()?;
        let workspace = record.workspace.as_mut().ok_or("owner workspace missing")?;
        operation(workspace, owner_token)?;
        // Never stop a transport for a mutation that failed to commit.
        commit_current_record(&record)?;
        // Hold the same transaction while draining: a concurrent republish
        // cannot register its new-epoch visitors before this drain.
        Ok(visitors.take_for_revocation())
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

/// One final in-process dispatch boundary. Never run provider I/O after
/// a committed owner revocation; owner changes serialize on this same lock.
/// The external call must be bounded, otherwise revocation liveness suffers.
fn guarded_dispatch_under_transaction<T>(
    transaction: &Mutex<()>,
    authorize: impl FnOnce() -> Result<(), String>,
    dispatch: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let _guard = transaction.lock();
    authorize()?;
    dispatch()
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
#[path = "rt1_workspace_service_tests.rs"]
mod tests;
