use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use vpr_integration::CancellationProbe;
use vpr_policy::{AuthorityScope, EffectiveAuthority, EgressDecision};

#[derive(Debug, Clone, Default)]
pub struct TurnCancellation {
    cancelled: Arc<AtomicBool>,
}

impl TurnCancellation {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

impl CancellationProbe for TurnCancellation {
    fn is_cancelled(&self) -> bool {
        self.is_cancelled()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeDenyReason {
    AuthorityDenied,
    EgressDenied,
    LocalOnlyRequired,
}

/// Authorizes a provider egress at the final runtime enforcement boundary.
///
/// # Errors
/// Returns `RuntimeDenyReason` when authority or egress policy forbids the call.
pub fn authorize_external_provider_call(
    authority: &EffectiveAuthority,
    required_scope: &AuthorityScope,
    egress: EgressDecision,
) -> Result<(), RuntimeDenyReason> {
    if !authority.allows(required_scope) {
        return Err(RuntimeDenyReason::AuthorityDenied);
    }
    match egress {
        EgressDecision::Allow(_) => Ok(()),
        EgressDecision::LocalOnly(_) => Err(RuntimeDenyReason::LocalOnlyRequired),
        EgressDecision::Deny(_) => Err(RuntimeDenyReason::EgressDenied),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpr_policy::{AuthorityLayer, EgressReason};

    #[test]
    fn cancellation_propagates_to_clones() {
        let authority = TurnCancellation::default();
        let provider_view = authority.clone();
        assert!(!CancellationProbe::is_cancelled(&provider_view));
        authority.cancel();
        assert!(CancellationProbe::is_cancelled(&provider_view));
    }

    #[test]
    fn local_only_policy_blocks_external_provider_even_when_scope_is_allowed() {
        let scope = AuthorityScope::new("provider.egress").unwrap();
        let authority = EffectiveAuthority::compose(&[AuthorityLayer::new([scope.clone()], [])]);
        let result = authorize_external_provider_call(
            &authority,
            &scope,
            EgressDecision::LocalOnly(EgressReason::LocalOnlyRequired),
        );
        assert_eq!(result, Err(RuntimeDenyReason::LocalOnlyRequired));
    }
}
