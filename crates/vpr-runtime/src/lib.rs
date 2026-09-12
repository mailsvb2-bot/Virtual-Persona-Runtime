use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use vpr_domain::{
    CorrelationId, OutputDeliveryState, OutputEvidence, PersonaIdentity, Rt0ReasonCode, Turn,
    TurnExecutionSnapshot, TurnId, TurnState,
};
use vpr_integration::{CancellationProbe, ProviderErrorKind};
use vpr_policy::{
    AuthorityScope, AuthorizationSnapshot, AuthorizationState, AuthorizationValidityError,
    EffectiveAuthority, EgressDecision, EgressReason,
};

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
    AuthorizationRevoked,
    AuthorizationExpired,
    AuthorizationStale,
    AuthorityDenied,
    EgressDenied,
    ConsentRequired,
    LocalOnlyRequired,
}

impl RuntimeDenyReason {
    #[must_use]
    pub const fn reason_code(self) -> Rt0ReasonCode {
        match self {
            Self::AuthorizationRevoked | Self::AuthorizationStale => Rt0ReasonCode::AuthRevoked,
            Self::AuthorizationExpired => Rt0ReasonCode::AuthExpired,
            Self::AuthorityDenied => Rt0ReasonCode::AuthScopeDenied,
            Self::EgressDenied => Rt0ReasonCode::EgressDenied,
            Self::ConsentRequired => Rt0ReasonCode::ConsentRequired,
            Self::LocalOnlyRequired => Rt0ReasonCode::EgressLocalOnly,
        }
    }
}

impl From<AuthorizationValidityError> for RuntimeDenyReason {
    fn from(value: AuthorizationValidityError) -> Self {
        match value {
            AuthorizationValidityError::StaleEpoch => Self::AuthorizationStale,
            AuthorizationValidityError::Revoked => Self::AuthorizationRevoked,
            AuthorizationValidityError::Expired => Self::AuthorizationExpired,
        }
    }
}

/// Authorizes provider egress at the final runtime enforcement boundary.
///
/// Current authorization is checked immediately before the provider call, so a cached allow
/// cannot survive revocation, replacement, or lease expiry.
///
/// # Errors
/// Returns `RuntimeDenyReason` when authorization, authority scope, or egress policy forbids
/// the call.
pub fn authorize_external_provider_call(
    authorization_state: AuthorizationState,
    authorization_snapshot: AuthorizationSnapshot,
    now_millis: u64,
    authority: &EffectiveAuthority,
    required_scope: &AuthorityScope,
    egress: EgressDecision,
) -> Result<(), RuntimeDenyReason> {
    authorization_state.validate(authorization_snapshot, now_millis)?;
    if !authority.allows(required_scope) {
        return Err(RuntimeDenyReason::AuthorityDenied);
    }
    match egress {
        EgressDecision::Allow(_) => Ok(()),
        EgressDecision::LocalOnly(_) => Err(RuntimeDenyReason::LocalOnlyRequired),
        EgressDecision::Deny(EgressReason::ConsentRequired) => {
            Err(RuntimeDenyReason::ConsentRequired)
        }
        EgressDecision::Deny(_) => Err(RuntimeDenyReason::EgressDenied),
    }
}

#[must_use]
pub fn capture_turn_execution_snapshot(
    turn: &Turn,
    persona: &PersonaIdentity,
    authorization: AuthorizationSnapshot,
) -> TurnExecutionSnapshot {
    TurnExecutionSnapshot::new(
        turn.id().clone(),
        turn.correlation_id().clone(),
        persona.id().clone(),
        persona.version(),
        persona.mode(),
        authorization.epoch(),
    )
}

#[derive(Debug, Clone)]
pub struct ActiveTurn {
    turn: Turn,
    snapshot: TurnExecutionSnapshot,
    cancellation: TurnCancellation,
    output: OutputEvidence,
}

impl ActiveTurn {
    #[must_use]
    pub fn new(
        turn_id: TurnId,
        correlation_id: CorrelationId,
        persona: &PersonaIdentity,
        authorization: AuthorizationSnapshot,
    ) -> Self {
        let turn = Turn::new(turn_id, correlation_id);
        let snapshot = capture_turn_execution_snapshot(&turn, persona, authorization);
        Self {
            turn,
            snapshot,
            cancellation: TurnCancellation::default(),
            output: OutputEvidence::default(),
        }
    }

    #[must_use]
    pub fn state(&self) -> TurnState {
        self.turn.state()
    }

    #[must_use]
    pub fn snapshot(&self) -> &TurnExecutionSnapshot {
        &self.snapshot
    }

    #[must_use]
    pub fn cancellation(&self) -> TurnCancellation {
        self.cancellation.clone()
    }

    #[must_use]
    pub fn output(&self) -> OutputEvidence {
        self.output
    }

    /// Marks a live turn as interrupted and propagates cancellation to provider views.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` if the turn or output evidence is already terminal.
    pub fn interrupt(&mut self) -> Result<(), Rt0ReasonCode> {
        let turn_interruptible = matches!(
            self.turn.state(),
            TurnState::Received
                | TurnState::Authorized
                | TurnState::Processing
                | TurnState::Outputting
        );
        let output_interruptible =
            !matches!(self.output.state(), OutputDeliveryState::Cancelled { .. });
        if !turn_interruptible || !output_interruptible {
            return Err(Rt0ReasonCode::InvalidStateTransition);
        }

        self.turn
            .transition(TurnState::Cancelled)
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)?;
        self.output
            .mark_cancelled()
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)?;
        self.cancellation.cancel();
        Ok(())
    }

    /// Advances the canonical turn lifecycle.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` for an invalid transition.
    pub fn transition(&mut self, next: TurnState) -> Result<(), Rt0ReasonCode> {
        self.turn
            .transition(next)
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    /// Records generated output.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` on invalid output ordering.
    pub fn mark_output_generated(&mut self) -> Result<(), Rt0ReasonCode> {
        self.output
            .mark_generated()
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    /// Records transport send.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` on invalid output ordering.
    pub fn mark_output_sent(&mut self) -> Result<(), Rt0ReasonCode> {
        self.output
            .mark_sent()
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    /// Records actual playback.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` on invalid output ordering.
    pub fn mark_output_played(&mut self) -> Result<(), Rt0ReasonCode> {
        self.output
            .mark_played()
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }
}

#[must_use]
pub const fn provider_reason_code(kind: ProviderErrorKind) -> Rt0ReasonCode {
    match kind {
        ProviderErrorKind::Unavailable => Rt0ReasonCode::ProviderUnavailable,
        ProviderErrorKind::RateLimited => Rt0ReasonCode::ProviderRateLimited,
        ProviderErrorKind::Timeout => Rt0ReasonCode::ProviderTimeout,
        ProviderErrorKind::Cancelled => Rt0ReasonCode::TurnCancelled,
        ProviderErrorKind::PolicyDenied => Rt0ReasonCode::EgressDenied,
        ProviderErrorKind::InvalidResponse => Rt0ReasonCode::InternalError,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpr_domain::{
        OutputCheckpoint, OutputDeliveryState, PersonaId, PersonaMode, PersonaVersion,
    };
    use vpr_policy::{AuthorityLayer, EgressReason};

    fn persona() -> PersonaIdentity {
        PersonaIdentity::new(
            PersonaId::new("persona-1").unwrap(),
            PersonaVersion::new(1).unwrap(),
            PersonaMode::DigitalTwin,
        )
    }

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
        let authorization = AuthorizationState::new(None);
        let result = authorize_external_provider_call(
            authorization,
            authorization.snapshot(),
            0,
            &authority,
            &scope,
            EgressDecision::LocalOnly(EgressReason::LocalOnlyRequired),
        );
        assert_eq!(result, Err(RuntimeDenyReason::LocalOnlyRequired));
    }

    #[test]
    fn cached_allow_is_rejected_after_revocation() {
        let scope = AuthorityScope::new("provider.egress").unwrap();
        let authority = EffectiveAuthority::compose(&[AuthorityLayer::new([scope.clone()], [])]);
        let mut authorization = AuthorizationState::new(None);
        let cached = authorization.snapshot();
        authorization.revoke().unwrap();

        let result = authorize_external_provider_call(
            authorization,
            cached,
            0,
            &authority,
            &scope,
            EgressDecision::Allow(EgressReason::Allowed),
        );

        assert_eq!(result, Err(RuntimeDenyReason::AuthorizationStale));
        assert_eq!(
            result.unwrap_err().reason_code(),
            Rt0ReasonCode::AuthRevoked
        );
    }

    #[test]
    fn execution_snapshot_binds_turn_persona_and_authorization_revision() {
        let authorization = AuthorizationState::new(None);
        let active = ActiveTurn::new(
            TurnId::new("turn-1").unwrap(),
            CorrelationId::new("corr-1").unwrap(),
            &persona(),
            authorization.snapshot(),
        );
        assert_eq!(active.snapshot().persona_id().as_str(), "persona-1");
        assert_eq!(active.snapshot().persona_version().get(), 1);
        assert_eq!(
            active.snapshot().authorization_epoch(),
            authorization.epoch()
        );
    }

    #[test]
    fn interruption_cancels_provider_view_and_preserves_unplayed_tail() {
        let authorization = AuthorizationState::new(None);
        let mut active = ActiveTurn::new(
            TurnId::new("turn-2").unwrap(),
            CorrelationId::new("corr-2").unwrap(),
            &persona(),
            authorization.snapshot(),
        );
        let provider_view = active.cancellation();
        active.transition(TurnState::Authorized).unwrap();
        active.transition(TurnState::Processing).unwrap();
        active.transition(TurnState::Outputting).unwrap();
        active.mark_output_generated().unwrap();
        active.mark_output_sent().unwrap();

        active.interrupt().unwrap();

        assert_eq!(active.state(), TurnState::Cancelled);
        assert!(provider_view.is_cancelled());
        assert_eq!(
            active.output().state(),
            OutputDeliveryState::Cancelled {
                reached: OutputCheckpoint::Sent
            }
        );
        assert!(!active.output().eligible_as_spoken());
    }

    #[test]
    fn invalid_interrupt_does_not_partially_mutate_output_evidence() {
        let authorization = AuthorizationState::new(None);
        let mut active = ActiveTurn::new(
            TurnId::new("turn-terminal").unwrap(),
            CorrelationId::new("corr-terminal").unwrap(),
            &persona(),
            authorization.snapshot(),
        );
        active.transition(TurnState::Denied).unwrap();
        assert_eq!(
            active.interrupt(),
            Err(Rt0ReasonCode::InvalidStateTransition)
        );
        assert_eq!(active.output().state(), OutputDeliveryState::Pending);
        assert!(!active.cancellation().is_cancelled());
    }

    #[test]
    fn missing_consent_uses_stable_consent_reason_code() {
        let scope = AuthorityScope::new("provider.egress").unwrap();
        let authority = EffectiveAuthority::compose(&[AuthorityLayer::new([scope.clone()], [])]);
        let authorization = AuthorizationState::new(None);
        let denied = authorize_external_provider_call(
            authorization,
            authorization.snapshot(),
            0,
            &authority,
            &scope,
            EgressDecision::Deny(EgressReason::ConsentRequired),
        );
        assert_eq!(denied, Err(RuntimeDenyReason::ConsentRequired));
        assert_eq!(
            denied.unwrap_err().reason_code(),
            Rt0ReasonCode::ConsentRequired
        );
    }

    #[test]
    fn provider_failures_map_to_stable_rt0_reason_codes() {
        assert_eq!(
            provider_reason_code(ProviderErrorKind::RateLimited),
            Rt0ReasonCode::ProviderRateLimited
        );
        assert_eq!(
            provider_reason_code(ProviderErrorKind::Cancelled),
            Rt0ReasonCode::TurnCancelled
        );
    }
}
