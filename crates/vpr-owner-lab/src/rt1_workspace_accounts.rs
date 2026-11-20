//! Owner-isolated mutations inside the ONE existing canonical Persona envelope.
//! The multi-owner enrollment entry is deliberately internal: a public signup
//! must first bind a separately reviewed Persona and non-duplicated provider
//! engine. Never create a second account registry or independent file store.
use vpr_policy::publication_lifecycle::PublicationStatus;

use super::{Rt1WorkspaceService, WorkspaceStatus};
use crate::owner_context::{DurableReviewedOwnerContextSnapshot, ReviewedOwnerContext};
use crate::persona_persistence::{
    AdditionalOwnerPersona, PersistedPersonaV2, commit_current_record, load_current_record,
};
use crate::persona_transaction;
use crate::rt1_visitor_session_manager::VisitorSessionManager;
use crate::rt1_workspace::OwnerWorkspaceRecord;

fn status_for(record: &PersistedPersonaV2, owner_token: &str) -> Result<WorkspaceStatus, String> {
    // Owner tokens are looked up only against server-stored credential hashes.
    // Neither owner_id nor Persona identity comes from the HTTP request.
    if let Some(primary) = &record.workspace {
        if primary.authenticate_owner(owner_token) {
            return Ok(workspace_status(
                primary,
                &record.snapshot.persona_id,
                record.snapshot.persona_version,
            ));
        }
    }
    for additional in record.additional_owner_personas.values() {
        if additional.workspace.authenticate_owner(owner_token) {
            return Ok(workspace_status(
                &additional.workspace,
                &additional.snapshot.persona_id,
                additional.snapshot.persona_version,
            ));
        }
    }
    Err("owner authorization denied".into())
}

fn workspace_status(
    workspace: &OwnerWorkspaceRecord,
    persona_id: &str,
    persona_version: u64,
) -> WorkspaceStatus {
    let publication_status = workspace.status().map(|status| match status {
        PublicationStatus::Private => "private",
        PublicationStatus::Published => "published",
        PublicationStatus::Paused => "paused",
        PublicationStatus::Unpublished => "unpublished",
    });
    WorkspaceStatus {
        owner_id: workspace.owner_id.clone(),
        persona_id: persona_id.to_owned(),
        persona_version,
        publication_status,
    }
}

fn provision_in_record(
    record: &mut PersistedPersonaV2,
    snapshot: DurableReviewedOwnerContextSnapshot,
) -> Result<(String, String), String> {
    ReviewedOwnerContext::from_durable_snapshot(&snapshot)
        .map_err(|_| "additional reviewed Persona is invalid")?;
    if snapshot.persona_id == record.snapshot.persona_id
        || record
            .additional_owner_personas
            .values()
            .any(|p| p.snapshot.persona_id == snapshot.persona_id)
    {
        return Err("Persona identity already belongs to an owner".into());
    }
    let (workspace, owner_token) = OwnerWorkspaceRecord::provision()?;
    let owner_id = workspace.owner_id.clone();
    if record.additional_owner_personas.contains_key(&owner_id)
        || record
            .workspace
            .as_ref()
            .is_some_and(|primary| primary.owner_id == owner_id)
    {
        return Err("owner identity collision".into());
    }
    record.additional_owner_personas.insert(
        owner_id.clone(),
        AdditionalOwnerPersona {
            snapshot,
            workspace,
        },
    );
    record.clone().validate()?;
    Ok((owner_id, owner_token))
}

fn correction_in_record(
    record: &mut PersistedPersonaV2,
    owner_token: &str,
    next: DurableReviewedOwnerContextSnapshot,
) -> Result<String, String> {
    ReviewedOwnerContext::from_durable_snapshot(&next)
        .map_err(|_| "reviewed Persona correction invalid")?;
    let (owner_id, owned) = record
        .additional_owner_personas
        .iter_mut()
        .find(|(_, owned)| owned.workspace.authenticate_owner(owner_token))
        .ok_or("owner authorization denied")?;
    if owned.snapshot.persona_id != next.persona_id
        || next.persona_version <= owned.snapshot.persona_version
    {
        return Err("cross-owner or nonmonotonic Persona correction denied".into());
    }
    owned
        .workspace
        .on_persona_correction(next.persona_version)?;
    owned.snapshot = next;
    let owner_id = owner_id.clone();
    record.clone().validate()?;
    Ok(owner_id)
}

impl Rt1WorkspaceService {
    /// Bind the exact visitor bearer to its owner and reviewed Persona before
    /// choosing a credential profile or allocating a provider/worker session.
    ///
    /// # Errors
    /// Rejects invalid or revoked visitor credentials and invalid canonical state.
    pub fn owner_binding_for_visitor(
        &self,
        bearer_token: &str,
    ) -> Result<super::VisitorOwnerBinding, String> {
        crate::persona_store_lease::require_live_runtime_owner()?;
        let _tx = persona_transaction::begin_workspace(self.transaction)?;
        let record = super::required_persona()?;
        let permit =
            super::admit_visitor_in_record(&record, bearer_token, super::server_now_millis()?)?;
        let (_, persona_id, persona_version) =
            super::published_visitor_workspace(&record, &permit)?;
        Ok(super::VisitorOwnerBinding {
            owner_id: permit.owner_id().to_owned(),
            persona_id: persona_id.to_owned(),
            persona_version,
            is_primary: record
                .workspace
                .as_ref()
                .is_some_and(|primary| primary.owner_id == permit.owner_id()),
        })
    }

    /// Existing single-owner route now understands exactly authenticated
    /// additional owners, without treating browser identity as authority.
    pub(super) fn status_for_authenticated_owner(
        &self,
        owner_token: &str,
    ) -> Result<WorkspaceStatus, String> {
        let _tx = persona_transaction::begin_workspace(self.transaction)?;
        let record = load_current_record()?.ok_or("reviewed Persona missing")?;
        status_for(&record, owner_token)
    }

    /// Provision a second owner only from a separately reviewed canonical
    /// snapshot. No external route is permitted until private Persona capture
    /// and per-owner engine selection are fully wired.
    #[allow(dead_code)]
    pub(crate) fn provision_verified_additional_owner(
        &self,
        reviewed: DurableReviewedOwnerContextSnapshot,
    ) -> Result<(String, String), String> {
        let _tx = persona_transaction::begin_workspace(self.transaction)?;
        let mut record = load_current_record()?.ok_or("reviewed Persona missing")?;
        let credentials = provision_in_record(&mut record, reviewed)?;
        commit_current_record(&record)?;
        Ok(credentials)
    }

    /// A correction may mutate ONLY the authenticated owner's own Persona.
    /// Commit first, invalidate exact-owner permits under the transaction,
    /// then perform potentially blocking remote STOP after releasing the lock.
    #[allow(dead_code)]
    pub(crate) fn correct_verified_additional_owner(
        &self,
        owner_token: &str,
        reviewed: DurableReviewedOwnerContextSnapshot,
        visitors: &VisitorSessionManager,
    ) -> Result<(), String> {
        let pending = {
            let _tx = persona_transaction::begin_workspace(self.transaction)?;
            let mut record = load_current_record()?.ok_or("reviewed Persona missing")?;
            let owner_id = correction_in_record(&mut record, owner_token, reviewed)?;
            commit_current_record(&record)?;
            visitors.take_for_revocation_for_owner(&owner_id)
        };
        pending.revoke();
        Ok(())
    }
}

#[cfg(all(test, not(windows)))]
#[path = "rt1_workspace_accounts_tests.rs"]
mod tests;
