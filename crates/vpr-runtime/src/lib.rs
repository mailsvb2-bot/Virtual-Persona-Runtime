use std::sync::{
    Arc, RwLock,
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
    InvalidTurnState,
    InternalError,
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
            Self::InvalidTurnState => Rt0ReasonCode::InvalidStateTransition,
            Self::InternalError => Rt0ReasonCode::InternalError,
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

/// Shared canonical authorization owner for active runtime work.
///
/// Clones share the same state. Callers cannot authorize against a detached copy of the state.
#[derive(Debug, Clone)]
pub struct AuthorizationController {
    state: Arc<RwLock<AuthorizationState>>,
}

impl AuthorizationController {
    #[must_use]
    pub fn new(expires_at_millis: Option<u64>) -> Self {
        Self {
            state: Arc::new(RwLock::new(AuthorizationState::new(expires_at_millis))),
        }
    }

    /// Captures the current authority revision for immutable turn evidence.
    ///
    /// # Errors
    /// Returns `INTERNAL_ERROR` if the shared state lock is poisoned.
    pub fn snapshot(&self) -> Result<AuthorizationSnapshot, RuntimeDenyReason> {
        self.state
            .read()
            .map_err(|_| RuntimeDenyReason::InternalError)
            .map(|state| state.snapshot())
    }

    /// Revokes the active authority and invalidates all earlier snapshots.
    ///
    /// # Errors
    /// Returns `INTERNAL_ERROR` if state cannot be updated or the epoch is exhausted.
    pub fn revoke(&self) -> Result<(), RuntimeDenyReason> {
        let mut state = self
            .state
            .write()
            .map_err(|_| RuntimeDenyReason::InternalError)?;
        state.revoke().map_err(|_| RuntimeDenyReason::InternalError)
    }

    /// Replaces authority with a fresh active epoch.
    ///
    /// # Errors
    /// Returns `INTERNAL_ERROR` if state cannot be updated or the epoch is exhausted.
    pub fn replace(&self, expires_at_millis: Option<u64>) -> Result<(), RuntimeDenyReason> {
        let mut state = self
            .state
            .write()
            .map_err(|_| RuntimeDenyReason::InternalError)?;
        state
            .replace(expires_at_millis)
            .map_err(|_| RuntimeDenyReason::InternalError)
    }

    fn validate_bound(
        &self,
        snapshot: AuthorizationSnapshot,
        now_millis: u64,
    ) -> Result<(), RuntimeDenyReason> {
        self.state
            .read()
            .map_err(|_| RuntimeDenyReason::InternalError)?
            .validate(snapshot, now_millis)
            .map_err(Into::into)
    }

    fn authorize_bound_provider_call(
        &self,
        snapshot: AuthorizationSnapshot,
        now_millis: u64,
        authority: &EffectiveAuthority,
        required_scope: &AuthorityScope,
        egress: EgressDecision,
    ) -> Result<(), RuntimeDenyReason> {
        self.validate_bound(snapshot, now_millis)?;
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OutputSegmentId(u64);

impl OutputSegmentId {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputSegmentEvidence {
    id: OutputSegmentId,
    evidence: OutputEvidence,
}

impl OutputSegmentEvidence {
    #[must_use]
    pub const fn id(self) -> OutputSegmentId {
        self.id
    }

    #[must_use]
    pub fn state(self) -> OutputDeliveryState {
        self.evidence.state()
    }

    #[must_use]
    pub fn eligible_as_spoken(self) -> bool {
        self.evidence.eligible_as_spoken()
    }
}

#[derive(Debug, Clone)]
pub struct ActiveTurn {
    turn: Turn,
    snapshot: TurnExecutionSnapshot,
    authorization: AuthorizationController,
    authorization_snapshot: AuthorizationSnapshot,
    cancellation: TurnCancellation,
    output_segments: Vec<OutputSegmentEvidence>,
    next_segment_id: u64,
}

impl ActiveTurn {
    /// Creates a turn bound to the current revision of one shared authorization controller.
    ///
    /// # Errors
    /// Returns `INTERNAL_ERROR` if current authorization cannot be snapshotted.
    pub fn new(
        turn_id: TurnId,
        correlation_id: CorrelationId,
        persona: &PersonaIdentity,
        authorization: &AuthorizationController,
    ) -> Result<Self, RuntimeDenyReason> {
        let authorization_snapshot = authorization.snapshot()?;
        let turn = Turn::new(turn_id, correlation_id);
        let snapshot = TurnExecutionSnapshot::new(
            turn.id().clone(),
            turn.correlation_id().clone(),
            persona.id().clone(),
            persona.version(),
            persona.mode(),
            authorization_snapshot.epoch(),
        );
        Ok(Self {
            turn,
            snapshot,
            authorization: authorization.clone(),
            authorization_snapshot,
            cancellation: TurnCancellation::default(),
            output_segments: Vec::new(),
            next_segment_id: 1,
        })
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
    pub fn output_segments(&self) -> &[OutputSegmentEvidence] {
        &self.output_segments
    }

    /// Authorizes the turn against the exact authorization revision captured in its snapshot.
    ///
    /// # Errors
    /// Returns a stable authorization reason if the bound revision is no longer current.
    pub fn authorize(&mut self, now_millis: u64) -> Result<(), Rt0ReasonCode> {
        self.authorization
            .validate_bound(self.authorization_snapshot, now_millis)
            .map_err(RuntimeDenyReason::reason_code)?;
        self.turn
            .transition(TurnState::Authorized)
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    /// Enforces the current shared authorization state immediately before provider egress.
    ///
    /// # Errors
    /// Returns `RuntimeDenyReason` if the turn's bound authority is stale/revoked/expired or the
    /// current scope/egress policy forbids the call.
    pub fn authorize_external_provider_call(
        &self,
        now_millis: u64,
        authority: &EffectiveAuthority,
        required_scope: &AuthorityScope,
        egress: EgressDecision,
    ) -> Result<(), RuntimeDenyReason> {
        if !matches!(
            self.turn.state(),
            TurnState::Authorized | TurnState::Processing | TurnState::Outputting
        ) {
            return Err(RuntimeDenyReason::InvalidTurnState);
        }
        self.authorization.authorize_bound_provider_call(
            self.authorization_snapshot,
            now_millis,
            authority,
            required_scope,
            egress,
        )
    }

    /// Moves an authorized turn into processing.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` for an invalid lifecycle transition.
    pub fn begin_processing(&mut self) -> Result<(), Rt0ReasonCode> {
        self.turn
            .transition(TurnState::Processing)
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    /// Moves a processing turn into output delivery.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` for an invalid lifecycle transition.
    pub fn begin_output(&mut self) -> Result<(), Rt0ReasonCode> {
        self.turn
            .transition(TurnState::Outputting)
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    /// Completes a turn after output delivery.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` for an invalid lifecycle transition.
    pub fn complete(&mut self) -> Result<(), Rt0ReasonCode> {
        self.turn
            .transition(TurnState::Completed)
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    /// Opens a generated streamed output segment.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` unless the turn is currently `OUTPUTTING`.
    pub fn begin_output_segment(&mut self) -> Result<OutputSegmentId, Rt0ReasonCode> {
        self.require_outputting()?;
        let id = OutputSegmentId(self.next_segment_id);
        self.next_segment_id = self
            .next_segment_id
            .checked_add(1)
            .ok_or(Rt0ReasonCode::InternalError)?;
        let mut evidence = OutputEvidence::default();
        evidence
            .mark_generated()
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)?;
        self.output_segments
            .push(OutputSegmentEvidence { id, evidence });
        Ok(id)
    }

    /// Marks one generated segment as sent to transport.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` unless the turn is `OUTPUTTING` and the segment exists
    /// in the expected checkpoint.
    pub fn mark_output_sent(&mut self, id: OutputSegmentId) -> Result<(), Rt0ReasonCode> {
        self.require_outputting()?;
        self.segment_mut(id)?
            .evidence
            .mark_sent()
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    /// Marks one sent segment as actually played.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` unless the turn is `OUTPUTTING` and the segment exists
    /// in the expected checkpoint.
    pub fn mark_output_played(&mut self, id: OutputSegmentId) -> Result<(), Rt0ReasonCode> {
        self.require_outputting()?;
        self.segment_mut(id)?
            .evidence
            .mark_played()
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    /// Interrupts the active turn and freezes every streamed segment at its reached checkpoint.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` if the turn is already terminal.
    pub fn interrupt(&mut self) -> Result<(), Rt0ReasonCode> {
        let interruptible = matches!(
            self.turn.state(),
            TurnState::Received
                | TurnState::Authorized
                | TurnState::Processing
                | TurnState::Outputting
        );
        if !interruptible
            || self.output_segments.iter().any(|segment| {
                matches!(
                    segment.evidence.state(),
                    OutputDeliveryState::Cancelled { .. }
                )
            })
        {
            return Err(Rt0ReasonCode::InvalidStateTransition);
        }

        self.cancellation.cancel();
        self.turn
            .transition(TurnState::Cancelled)
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)?;
        for segment in &mut self.output_segments {
            segment
                .evidence
                .mark_cancelled()
                .map_err(|_| Rt0ReasonCode::InvalidStateTransition)?;
        }
        Ok(())
    }

    fn require_outputting(&self) -> Result<(), Rt0ReasonCode> {
        if self.turn.state() == TurnState::Outputting {
            Ok(())
        } else {
            Err(Rt0ReasonCode::InvalidStateTransition)
        }
    }

    fn segment_mut(
        &mut self,
        id: OutputSegmentId,
    ) -> Result<&mut OutputSegmentEvidence, Rt0ReasonCode> {
        self.output_segments
            .iter_mut()
            .find(|segment| segment.id == id)
            .ok_or(Rt0ReasonCode::InvalidStateTransition)
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
    use vpr_domain::{OutputCheckpoint, PersonaId, PersonaMode, PersonaVersion};
    use vpr_policy::{AuthorityLayer, EgressReason};

    fn persona() -> PersonaIdentity {
        PersonaIdentity::new(
            PersonaId::new("persona-1").unwrap(),
            PersonaVersion::new(1).unwrap(),
            PersonaMode::DigitalTwin,
        )
    }

    fn allowed_provider_authority() -> (AuthorityScope, EffectiveAuthority) {
        let scope = AuthorityScope::new("provider.egress").unwrap();
        let authority = EffectiveAuthority::compose(&[AuthorityLayer::new([scope.clone()], [])]);
        (scope, authority)
    }

    #[test]
    fn cancellation_propagates_to_clones() {
        let cancellation = TurnCancellation::default();
        let provider_view = cancellation.clone();
        assert!(!CancellationProbe::is_cancelled(&provider_view));
        cancellation.cancel();
        assert!(CancellationProbe::is_cancelled(&provider_view));
    }

    #[test]
    fn provider_call_before_turn_authorization_is_rejected() {
        let (scope, authority) = allowed_provider_authority();
        let authorization = AuthorizationController::new(None);
        let turn = ActiveTurn::new(
            TurnId::new("turn-premature").unwrap(),
            CorrelationId::new("corr-premature").unwrap(),
            &persona(),
            &authorization,
        )
        .unwrap();

        assert_eq!(
            turn.authorize_external_provider_call(
                0,
                &authority,
                &scope,
                EgressDecision::Allow(EgressReason::Allowed),
            ),
            Err(RuntimeDenyReason::InvalidTurnState)
        );
    }

    #[test]
    fn local_only_policy_blocks_external_provider_even_when_scope_is_allowed() {
        let (scope, authority) = allowed_provider_authority();
        let authorization = AuthorizationController::new(None);
        let mut turn = ActiveTurn::new(
            TurnId::new("turn-local").unwrap(),
            CorrelationId::new("corr-local").unwrap(),
            &persona(),
            &authorization,
        )
        .unwrap();
        turn.authorize(0).unwrap();
        let result = turn.authorize_external_provider_call(
            0,
            &authority,
            &scope,
            EgressDecision::LocalOnly(EgressReason::LocalOnlyRequired),
        );
        assert_eq!(result, Err(RuntimeDenyReason::LocalOnlyRequired));
    }

    #[test]
    fn shared_revocation_invalidates_every_controller_clone_and_bound_turn() {
        let (scope, authority) = allowed_provider_authority();
        let authorization = AuthorizationController::new(None);
        let revoker = authorization.clone();
        let mut turn = ActiveTurn::new(
            TurnId::new("turn-revoke").unwrap(),
            CorrelationId::new("corr-revoke").unwrap(),
            &persona(),
            &authorization,
        )
        .unwrap();
        turn.authorize(0).unwrap();
        revoker.revoke().unwrap();

        let result = turn.authorize_external_provider_call(
            1,
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
    fn replaced_authority_cannot_be_used_with_old_turn_snapshot() {
        let (scope, authority) = allowed_provider_authority();
        let authorization = AuthorizationController::new(None);
        let mut turn = ActiveTurn::new(
            TurnId::new("turn-old-epoch").unwrap(),
            CorrelationId::new("corr-old-epoch").unwrap(),
            &persona(),
            &authorization,
        )
        .unwrap();
        turn.authorize(0).unwrap();
        authorization.replace(None).unwrap();

        assert_eq!(
            turn.authorize_external_provider_call(
                0,
                &authority,
                &scope,
                EgressDecision::Allow(EgressReason::Allowed),
            ),
            Err(RuntimeDenyReason::AuthorizationStale)
        );
    }

    #[test]
    fn execution_snapshot_is_the_same_epoch_enforced_for_provider_calls() {
        let authorization = AuthorizationController::new(None);
        let turn = ActiveTurn::new(
            TurnId::new("turn-snapshot").unwrap(),
            CorrelationId::new("corr-snapshot").unwrap(),
            &persona(),
            &authorization,
        )
        .unwrap();
        assert_eq!(
            turn.snapshot().authorization_epoch(),
            authorization.snapshot().unwrap().epoch()
        );
    }

    #[test]
    fn interruption_preserves_played_prefix_and_unplayed_tail_per_segment() {
        let authorization = AuthorizationController::new(None);
        let mut turn = ActiveTurn::new(
            TurnId::new("turn-stream").unwrap(),
            CorrelationId::new("corr-stream").unwrap(),
            &persona(),
            &authorization,
        )
        .unwrap();
        let provider_view = turn.cancellation();
        turn.authorize(0).unwrap();
        turn.begin_processing().unwrap();
        turn.begin_output().unwrap();

        let spoken = turn.begin_output_segment().unwrap();
        turn.mark_output_sent(spoken).unwrap();
        turn.mark_output_played(spoken).unwrap();
        let tail = turn.begin_output_segment().unwrap();
        turn.mark_output_sent(tail).unwrap();

        turn.interrupt().unwrap();

        assert_eq!(turn.state(), TurnState::Cancelled);
        assert!(provider_view.is_cancelled());
        let segments = turn.output_segments();
        assert_eq!(segments.len(), 2);
        assert_eq!(
            segments[0].state(),
            OutputDeliveryState::Cancelled {
                reached: OutputCheckpoint::Played
            }
        );
        assert!(segments[0].eligible_as_spoken());
        assert_eq!(
            segments[1].state(),
            OutputDeliveryState::Cancelled {
                reached: OutputCheckpoint::Sent
            }
        );
        assert!(!segments[1].eligible_as_spoken());
    }

    #[test]
    fn denied_turn_cannot_emit_output_evidence() {
        let authorization = AuthorizationController::new(None);
        let mut turn = ActiveTurn::new(
            TurnId::new("turn-denied").unwrap(),
            CorrelationId::new("corr-denied").unwrap(),
            &persona(),
            &authorization,
        )
        .unwrap();
        turn.turn.transition(TurnState::Denied).unwrap();
        assert_eq!(
            turn.begin_output_segment(),
            Err(Rt0ReasonCode::InvalidStateTransition)
        );
        assert!(turn.output_segments().is_empty());
    }

    #[test]
    fn invalid_interrupt_does_not_partially_mutate_output_evidence() {
        let authorization = AuthorizationController::new(None);
        let mut turn = ActiveTurn::new(
            TurnId::new("turn-terminal").unwrap(),
            CorrelationId::new("corr-terminal").unwrap(),
            &persona(),
            &authorization,
        )
        .unwrap();
        turn.turn.transition(TurnState::Denied).unwrap();
        assert_eq!(turn.interrupt(), Err(Rt0ReasonCode::InvalidStateTransition));
        assert!(turn.output_segments().is_empty());
        assert!(!turn.cancellation().is_cancelled());
    }

    #[test]
    fn missing_consent_uses_stable_consent_reason_code() {
        let (scope, authority) = allowed_provider_authority();
        let authorization = AuthorizationController::new(None);
        let mut turn = ActiveTurn::new(
            TurnId::new("turn-consent").unwrap(),
            CorrelationId::new("corr-consent").unwrap(),
            &persona(),
            &authorization,
        )
        .unwrap();
        turn.authorize(0).unwrap();
        let denied = turn.authorize_external_provider_call(
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
