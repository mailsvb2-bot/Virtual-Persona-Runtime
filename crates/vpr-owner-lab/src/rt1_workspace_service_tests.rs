//! Regression tests for the canonical RT1 workspace transaction boundary.
//! Kept separate from the production service to preserve the God File guard.

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
    let next_permit = admit_from_workspace(&workspace, &next_visitor, "persona-a", 3, 4).unwrap();
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

    let mut publication = CanonicalPublication::new("owner", "persona", "publication", 1).unwrap();
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

#[cfg(not(windows))]
#[test]
fn local_delivery_does_not_enqueue_without_authorized_store() {
    use vpr_policy::publication_lifecycle::CanonicalPublication;

    let mut publication = CanonicalPublication::new("owner", "persona", "publication", 1).unwrap();
    publication.publish("owner", 1, true).unwrap();
    let permit = publication.visitor_lease(true).unwrap();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let service = Rt1WorkspaceService::new();
    let result = service.enqueue_authorized_visitor_fragment("visitor", &permit, &sender, "hello");
    assert!(result.is_err());
    assert!(receiver.try_recv().is_err());
}

#[test]
fn local_delivery_rejects_revoked_authority_and_full_queue() {
    let (mut workspace, owner) = OwnerWorkspaceRecord::provision().unwrap();
    let visitor = workspace.publish(&owner, "persona-a", 3, true).unwrap();
    let permit = workspace.visitor_permit(&visitor).unwrap();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);

    assert!(
        enqueue_from_workspace(
            &workspace,
            (&visitor, &permit),
            ("persona-a", 3),
            10,
            &sender,
            "first",
        )
        .is_ok()
    );
    // A full bounded queue must fail immediately, never wait under the
    // owner publication transaction or drop the first authorized item.
    assert!(
        enqueue_from_workspace(
            &workspace,
            (&visitor, &permit),
            ("persona-a", 3),
            10,
            &sender,
            "second",
        )
        .is_err()
    );
    assert_eq!(receiver.try_recv().unwrap(), "first");

    workspace.pause(&owner).unwrap();
    assert!(
        enqueue_from_workspace(
            &workspace,
            (&visitor, &permit),
            ("persona-a", 3),
            11,
            &sender,
            "revoked",
        )
        .is_err()
    );
    assert!(receiver.try_recv().is_err());

    let new_visitor = workspace.publish(&owner, "persona-a", 3, true).unwrap();
    let new_permit = workspace.visitor_permit(&new_visitor).unwrap();
    assert!(
        enqueue_from_workspace(
            &workspace,
            (&visitor, &permit),
            ("persona-a", 3),
            12,
            &sender,
            "stale",
        )
        .is_err()
    );
    assert!(
        enqueue_from_workspace(
            &workspace,
            (&new_visitor, &new_permit),
            ("persona-a", 3),
            12,
            &sender,
            "fresh",
        )
        .is_ok()
    );
    assert_eq!(receiver.try_recv().unwrap(), "fresh");
}

#[test]
fn independent_service_handles_share_one_transaction_lock() {
    let first = Rt1WorkspaceService::new();
    let second = Rt1WorkspaceService::new();
    assert!(std::ptr::eq(first.transaction, second.transaction));
    let held = first.transaction.lock();
    assert!(second.transaction.try_lock().is_none());
    drop(held);
    assert!(second.transaction.try_lock().is_some());
}

#[test]
fn service_signature_does_not_accept_client_claimed_readiness() {
    // Browser JS cannot invoke `publish` without the server-provided
    // readiness gate; all calls go through the service transaction.
    let _ = Rt1WorkspaceService::new();
    assert!(required_persona().is_err());
}

#[cfg(not(windows))]
#[test]
fn provider_dispatch_never_calls_remote_provider_without_canonical_store() {
    use std::cell::Cell;
    use vpr_policy::publication_lifecycle::CanonicalPublication;

    let mut publication = CanonicalPublication::new("owner", "persona", "publication", 1).unwrap();
    publication.publish("owner", 1, true).unwrap();
    let permit = publication.visitor_lease(true).unwrap();
    let called = Cell::new(false);
    let result = Rt1WorkspaceService::new().guarded_provider_dispatch("visitor", &permit, || {
        called.set(true);
        Ok(())
    });
    assert!(result.is_err());
    assert!(
        !called.get(),
        "provider must not run without canonical authority"
    );
}
