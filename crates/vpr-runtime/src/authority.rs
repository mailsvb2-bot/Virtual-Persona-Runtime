use std::sync::{
    Arc, RwLock, Weak,
    atomic::{AtomicBool, Ordering},
};

use vpr_domain::PolicyRevision;
use vpr_policy::{
    AuthorityScope, AuthorizationSnapshot, AuthorizationState, ConsentState, DataClass,
    EffectiveAuthority, EgressDecision, EgressReason, EgressRequest, decide_egress,
};

use crate::cancellation::TurnCancellation;
use crate::error::RuntimeDenyReason;
use crate::provider::ProviderExecutionPermit;

#[derive(Debug)]
struct BoundTurnCancellation {
    epoch: vpr_domain::AuthorizationEpoch,
    cancellation: Weak<AtomicBool>,
}

#[derive(Debug)]
struct AuthorizationRuntimeState {
    authorization: AuthorizationState,
    effective_authority: EffectiveAuthority,
    bound_turns: Vec<BoundTurnCancellation>,
}

/// Shared canonical authorization owner for active runtime work.
///
/// Clones share the same state. Binding a turn and revoking/replacing authority are serialized
/// through the same state boundary, so revocation cannot miss a concurrently created turn.
#[derive(Debug, Clone)]
pub(crate) struct AuthorizationController {
    state: Arc<RwLock<AuthorizationRuntimeState>>,
}

pub(crate) struct ProviderCallContext<'a> {
    pub(crate) egress_policy: &'a EgressPolicyController,
    pub(crate) egress_snapshot: EgressPolicySnapshot,
    pub(crate) cancellation: &'a TurnCancellation,
    pub(crate) required_scope: &'a AuthorityScope,
    pub(crate) data_class: DataClass,
}

impl AuthorizationController {
    #[must_use]
    pub(crate) fn new(
        expires_at_millis: Option<u64>,
        effective_authority: EffectiveAuthority,
    ) -> Self {
        Self {
            state: Arc::new(RwLock::new(AuthorizationRuntimeState {
                authorization: AuthorizationState::new(expires_at_millis),
                effective_authority,
                bound_turns: Vec::new(),
            })),
        }
    }

    pub(crate) fn bind_turn(
        &self,
        cancellation: &TurnCancellation,
    ) -> Result<AuthorizationSnapshot, RuntimeDenyReason> {
        let mut state = self
            .state
            .write()
            .map_err(|_| RuntimeDenyReason::InternalError)?;
        state
            .bound_turns
            .retain(|bound| bound.cancellation.strong_count() > 0);
        let snapshot = state.authorization.snapshot();
        state.bound_turns.push(BoundTurnCancellation {
            epoch: snapshot.epoch(),
            cancellation: cancellation.downgrade(),
        });
        Ok(snapshot)
    }

    fn cancel_stale_bound_turns(state: &mut AuthorizationRuntimeState) {
        let current_epoch = state.authorization.epoch();
        state.bound_turns.retain(|bound| {
            let Some(cancellation) = bound.cancellation.upgrade() else {
                return false;
            };
            if bound.epoch != current_epoch {
                cancellation.store(true, Ordering::Release);
            }
            true
        });
    }

    /// Revokes the active authority, invalidates earlier snapshots, and cancels bound active work.
    ///
    /// # Errors
    /// Returns `INTERNAL_ERROR` if state cannot be updated or the epoch is exhausted.
    pub(crate) fn revoke(&self) -> Result<(), RuntimeDenyReason> {
        let mut state = self
            .state
            .write()
            .map_err(|_| RuntimeDenyReason::InternalError)?;
        state
            .authorization
            .revoke()
            .map_err(|_| RuntimeDenyReason::InternalError)?;
        Self::cancel_stale_bound_turns(&mut state);
        Ok(())
    }

    /// Replaces authority with a fresh active epoch and cancels work bound to the prior epoch.
    ///
    /// # Errors
    /// Returns `INTERNAL_ERROR` if state cannot be updated or the epoch is exhausted.
    pub(crate) fn replace(&self, expires_at_millis: Option<u64>) -> Result<(), RuntimeDenyReason> {
        let mut state = self
            .state
            .write()
            .map_err(|_| RuntimeDenyReason::InternalError)?;
        state
            .authorization
            .replace(expires_at_millis)
            .map_err(|_| RuntimeDenyReason::InternalError)?;
        Self::cancel_stale_bound_turns(&mut state);
        Ok(())
    }

    pub(crate) fn replace_authority(
        &self,
        effective_authority: EffectiveAuthority,
        expires_at_millis: Option<u64>,
    ) -> Result<(), RuntimeDenyReason> {
        let mut state = self
            .state
            .write()
            .map_err(|_| RuntimeDenyReason::InternalError)?;
        state
            .authorization
            .replace(expires_at_millis)
            .map_err(|_| RuntimeDenyReason::InternalError)?;
        state.effective_authority = effective_authority;
        Self::cancel_stale_bound_turns(&mut state);
        Ok(())
    }

    pub(crate) fn validate_bound(
        &self,
        snapshot: AuthorizationSnapshot,
        now_millis: u64,
    ) -> Result<(), RuntimeDenyReason> {
        self.state
            .read()
            .map_err(|_| RuntimeDenyReason::InternalError)?
            .authorization
            .validate(snapshot, now_millis)
            .map_err(RuntimeDenyReason::from)
    }

    pub(crate) fn issue_provider_permit(
        &self,
        snapshot: AuthorizationSnapshot,
        now_millis: u64,
        context: &ProviderCallContext<'_>,
    ) -> Result<ProviderExecutionPermit, RuntimeDenyReason> {
        let authorization_state = self
            .state
            .read()
            .map_err(|_| RuntimeDenyReason::InternalError)?;
        let policy_state = context
            .egress_policy
            .state
            .read()
            .map_err(|_| RuntimeDenyReason::InternalError)?;
        authorization_state
            .authorization
            .validate(snapshot, now_millis)
            .map_err(RuntimeDenyReason::from)?;
        if policy_state.revision != context.egress_snapshot.revision {
            return Err(RuntimeDenyReason::EgressPolicyStale);
        }
        if context.cancellation.is_cancelled() {
            return Err(RuntimeDenyReason::TurnCancelled);
        }
        if !authorization_state
            .effective_authority
            .allows(context.required_scope)
        {
            return Err(RuntimeDenyReason::AuthorityDenied);
        }
        let egress = decide_egress(EgressRequest {
            data_class: context.data_class,
            provider_policy_allows: policy_state.provider_policy_allows,
            consent: policy_state.consent,
            local_only_required: policy_state.local_only_required,
        });
        match egress {
            EgressDecision::Allow(_) => Ok(ProviderExecutionPermit {
                cancellation: context.cancellation.clone(),
            }),
            EgressDecision::LocalOnly(_) => Err(RuntimeDenyReason::LocalOnlyRequired),
            EgressDecision::Deny(EgressReason::ConsentRequired) => {
                Err(RuntimeDenyReason::ConsentRequired)
            }
            EgressDecision::Deny(_) => Err(RuntimeDenyReason::EgressDenied),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EgressPolicySnapshot {
    pub(crate) revision: PolicyRevision,
}

#[derive(Debug)]
struct EgressPolicyRuntimeState {
    revision: PolicyRevision,
    provider_policy_allows: bool,
    consent: ConsentState,
    local_only_required: bool,
    bound_turns: Vec<(PolicyRevision, Weak<AtomicBool>)>,
}

#[derive(Debug, Clone)]
pub(crate) struct EgressPolicyController {
    state: Arc<RwLock<EgressPolicyRuntimeState>>,
}

impl EgressPolicyController {
    #[must_use]
    pub(crate) fn new(
        provider_policy_allows: bool,
        consent: ConsentState,
        local_only_required: bool,
    ) -> Self {
        Self {
            state: Arc::new(RwLock::new(EgressPolicyRuntimeState {
                revision: PolicyRevision::initial(),
                provider_policy_allows,
                consent,
                local_only_required,
                bound_turns: Vec::new(),
            })),
        }
    }

    pub(crate) fn bind_turn(
        &self,
        cancellation: &TurnCancellation,
    ) -> Result<EgressPolicySnapshot, RuntimeDenyReason> {
        let mut state = self
            .state
            .write()
            .map_err(|_| RuntimeDenyReason::InternalError)?;
        state
            .bound_turns
            .retain(|(_, weak)| weak.strong_count() > 0);
        let snapshot = EgressPolicySnapshot {
            revision: state.revision,
        };
        state
            .bound_turns
            .push((snapshot.revision, cancellation.downgrade()));
        Ok(snapshot)
    }

    pub(crate) fn validate_snapshot(
        &self,
        snapshot: EgressPolicySnapshot,
    ) -> Result<(), RuntimeDenyReason> {
        let state = self
            .state
            .read()
            .map_err(|_| RuntimeDenyReason::InternalError)?;
        if state.revision == snapshot.revision {
            Ok(())
        } else {
            Err(RuntimeDenyReason::EgressPolicyStale)
        }
    }

    fn advance_and_cancel(state: &mut EgressPolicyRuntimeState) -> Result<(), RuntimeDenyReason> {
        state.revision = state
            .revision
            .next()
            .map_err(|_| RuntimeDenyReason::InternalError)?;
        let current = state.revision;
        state.bound_turns.retain(|(revision, weak)| {
            let Some(cancellation) = weak.upgrade() else {
                return false;
            };
            if *revision != current {
                cancellation.store(true, Ordering::Release);
            }
            true
        });
        Ok(())
    }

    /// Updates provider-policy compatibility and invalidates turns bound to the old revision.
    ///
    /// # Errors
    /// Returns `INTERNAL_ERROR` when policy state cannot be updated.
    pub(crate) fn set_provider_policy_allows(&self, allows: bool) -> Result<(), RuntimeDenyReason> {
        let mut state = self
            .state
            .write()
            .map_err(|_| RuntimeDenyReason::InternalError)?;
        if state.provider_policy_allows != allows {
            Self::advance_and_cancel(&mut state)?;
            state.provider_policy_allows = allows;
        }
        Ok(())
    }

    /// Updates consent and invalidates turns bound to the old revision.
    ///
    /// # Errors
    /// Returns `INTERNAL_ERROR` when policy state cannot be updated.
    pub(crate) fn set_consent(&self, consent: ConsentState) -> Result<(), RuntimeDenyReason> {
        let mut state = self
            .state
            .write()
            .map_err(|_| RuntimeDenyReason::InternalError)?;
        if state.consent != consent {
            Self::advance_and_cancel(&mut state)?;
            state.consent = consent;
        }
        Ok(())
    }

    /// Updates the local-only constraint and invalidates turns bound to the old revision.
    ///
    /// # Errors
    /// Returns `INTERNAL_ERROR` when policy state cannot be updated.
    pub(crate) fn set_local_only_required(&self, required: bool) -> Result<(), RuntimeDenyReason> {
        let mut state = self
            .state
            .write()
            .map_err(|_| RuntimeDenyReason::InternalError)?;
        if state.local_only_required != required {
            Self::advance_and_cancel(&mut state)?;
            state.local_only_required = required;
        }
        Ok(())
    }
}
