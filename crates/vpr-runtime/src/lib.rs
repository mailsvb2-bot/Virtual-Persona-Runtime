use std::sync::{
    Arc, RwLock, Weak,
    atomic::{AtomicBool, Ordering},
};

use vpr_domain::{
    CorrelationId, OutputDeliveryState, OutputEvidence, PersonaIdentity, PolicyRevision,
    RealtimeSession, RealtimeSessionState, Rt0ReasonCode, SessionId, Turn, TurnExecutionSnapshot,
    TurnId, TurnState,
};
use vpr_integration::{CancellationProbe, ProviderErrorKind};
use vpr_policy::{
    AuthorityScope, AuthorizationSnapshot, AuthorizationState, AuthorizationValidityError,
    ConsentState, DataClass, EffectiveAuthority, EgressDecision, EgressReason, EgressRequest,
    decide_egress,
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

    fn downgrade(&self) -> Weak<AtomicBool> {
        Arc::downgrade(&self.cancelled)
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
    EgressPolicyStale,
    ConsentRequired,
    LocalOnlyRequired,
    InvalidTurnState,
    TurnCancelled,
    InternalError,
}

impl RuntimeDenyReason {
    #[must_use]
    pub const fn reason_code(self) -> Rt0ReasonCode {
        match self {
            Self::AuthorizationRevoked | Self::AuthorizationStale => Rt0ReasonCode::AuthRevoked,
            Self::AuthorizationExpired => Rt0ReasonCode::AuthExpired,
            Self::AuthorityDenied => Rt0ReasonCode::AuthScopeDenied,
            Self::EgressDenied | Self::EgressPolicyStale => Rt0ReasonCode::EgressDenied,
            Self::ConsentRequired => Rt0ReasonCode::ConsentRequired,
            Self::LocalOnlyRequired => Rt0ReasonCode::EgressLocalOnly,
            Self::InvalidTurnState => Rt0ReasonCode::InvalidStateTransition,
            Self::TurnCancelled => Rt0ReasonCode::TurnCancelled,
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

#[derive(Debug)]
struct BoundTurnCancellation {
    epoch: vpr_domain::AuthorizationEpoch,
    cancellation: Weak<AtomicBool>,
}

#[derive(Debug)]
struct AuthorizationRuntimeState {
    authorization: AuthorizationState,
    bound_turns: Vec<BoundTurnCancellation>,
}

/// Shared canonical authorization owner for active runtime work.
///
/// Clones share the same state. Binding a turn and revoking/replacing authority are serialized
/// through the same state boundary, so revocation cannot miss a concurrently created turn.
#[derive(Debug, Clone)]
struct AuthorizationController {
    state: Arc<RwLock<AuthorizationRuntimeState>>,
}

struct ProviderCallContext<'a> {
    egress_policy: &'a EgressPolicyController,
    egress_snapshot: EgressPolicySnapshot,
    cancellation: &'a TurnCancellation,
    now_millis: u64,
    authority: &'a EffectiveAuthority,
    required_scope: &'a AuthorityScope,
    data_class: DataClass,
}

impl AuthorizationController {
    #[must_use]
    fn new(expires_at_millis: Option<u64>) -> Self {
        Self {
            state: Arc::new(RwLock::new(AuthorizationRuntimeState {
                authorization: AuthorizationState::new(expires_at_millis),
                bound_turns: Vec::new(),
            })),
        }
    }

    fn bind_turn(
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
    fn revoke(&self) -> Result<(), RuntimeDenyReason> {
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
    fn replace(&self, expires_at_millis: Option<u64>) -> Result<(), RuntimeDenyReason> {
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

    fn issue_provider_permit(
        &self,
        snapshot: AuthorizationSnapshot,
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
            .validate(snapshot, context.now_millis)
            .map_err(RuntimeDenyReason::from)?;
        if policy_state.revision != context.egress_snapshot.revision {
            return Err(RuntimeDenyReason::EgressPolicyStale);
        }
        if context.cancellation.is_cancelled() {
            return Err(RuntimeDenyReason::TurnCancelled);
        }
        if !context.authority.allows(context.required_scope) {
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
                authorization_epoch: snapshot.epoch(),
                egress_policy_revision: policy_state.revision,
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
struct EgressPolicySnapshot {
    revision: PolicyRevision,
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
struct EgressPolicyController {
    state: Arc<RwLock<EgressPolicyRuntimeState>>,
}

impl EgressPolicyController {
    #[must_use]
    fn new(provider_policy_allows: bool, consent: ConsentState, local_only_required: bool) -> Self {
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

    fn bind_turn(
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
    fn set_provider_policy_allows(&self, allows: bool) -> Result<(), RuntimeDenyReason> {
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
    fn set_consent(&self, consent: ConsentState) -> Result<(), RuntimeDenyReason> {
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
    fn set_local_only_required(&self, required: bool) -> Result<(), RuntimeDenyReason> {
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

#[derive(Debug)]
pub struct ActiveSession {
    session: RealtimeSession,
    authorization: AuthorizationController,
    egress_policy: EgressPolicyController,
}

impl ActiveSession {
    #[must_use]
    pub fn new(
        id: SessionId,
        persona_id: vpr_domain::PersonaId,
        expires_at_millis: Option<u64>,
        provider_policy_allows: bool,
        consent: ConsentState,
        local_only_required: bool,
    ) -> Self {
        Self {
            session: RealtimeSession::new(id, persona_id),
            authorization: AuthorizationController::new(expires_at_millis),
            egress_policy: EgressPolicyController::new(
                provider_policy_allows,
                consent,
                local_only_required,
            ),
        }
    }

    #[must_use]
    pub fn state(&self) -> RealtimeSessionState {
        self.session.state()
    }

    #[must_use]
    pub fn id(&self) -> &SessionId {
        self.session.id()
    }

    /// Activates a newly created runtime session.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` unless the session can become active.
    pub fn activate(&mut self) -> Result<(), Rt0ReasonCode> {
        self.session
            .transition(RealtimeSessionState::Active)
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    /// Starts graceful session draining.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` unless the session is active.
    pub fn begin_draining(&mut self) -> Result<(), Rt0ReasonCode> {
        self.session
            .transition(RealtimeSessionState::Draining)
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    /// Revokes the session and its shared authorization before publishing the revoked state.
    ///
    /// # Errors
    /// Returns a stable RT0 reason if revocation cannot be applied safely.
    pub fn revoke(&mut self) -> Result<(), Rt0ReasonCode> {
        let mut candidate = self.session.clone();
        candidate
            .transition(RealtimeSessionState::Revoked)
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)?;
        self.authorization
            .revoke()
            .map_err(RuntimeDenyReason::reason_code)?;
        self.session = candidate;
        Ok(())
    }

    /// Closes a draining or revoked session and revokes any remaining authority.
    ///
    /// # Errors
    /// Returns a stable RT0 reason if closing cannot be applied safely.
    pub fn close(&mut self) -> Result<(), Rt0ReasonCode> {
        let mut candidate = self.session.clone();
        candidate
            .transition(RealtimeSessionState::Closed)
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)?;
        self.authorization
            .revoke()
            .map_err(RuntimeDenyReason::reason_code)?;
        self.session = candidate;
        Ok(())
    }

    /// Replaces current session consent.
    ///
    /// # Errors
    /// Returns a stable RT0 reason if policy state cannot be updated.
    pub fn set_consent(&self, consent: ConsentState) -> Result<(), Rt0ReasonCode> {
        self.egress_policy
            .set_consent(consent)
            .map_err(RuntimeDenyReason::reason_code)
    }

    /// Replaces current provider-policy compatibility.
    ///
    /// # Errors
    /// Returns a stable RT0 reason if policy state cannot be updated.
    pub fn set_provider_policy_allows(&self, allows: bool) -> Result<(), Rt0ReasonCode> {
        self.egress_policy
            .set_provider_policy_allows(allows)
            .map_err(RuntimeDenyReason::reason_code)
    }

    /// Replaces the current local-only requirement.
    ///
    /// # Errors
    /// Returns a stable RT0 reason if policy state cannot be updated.
    pub fn set_local_only_required(&self, required: bool) -> Result<(), Rt0ReasonCode> {
        self.egress_policy
            .set_local_only_required(required)
            .map_err(RuntimeDenyReason::reason_code)
    }

    /// Rotates the active authorization epoch and invalidates all work bound to the old epoch.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` unless the session is active, or a stable RT0 reason
    /// if the authoritative authorization state cannot be replaced safely.
    pub fn refresh_authorization(
        &self,
        expires_at_millis: Option<u64>,
    ) -> Result<(), Rt0ReasonCode> {
        if self.session.state() != RealtimeSessionState::Active {
            return Err(Rt0ReasonCode::InvalidStateTransition);
        }
        self.authorization
            .replace(expires_at_millis)
            .map_err(RuntimeDenyReason::reason_code)
    }
}

/// Linearized permission to start one provider operation.
///
/// Issuance is serialized against revoke/replace. A permit issued before revocation represents
/// already-started work and carries the same cancellation signal that revocation flips.
#[derive(Debug)]
pub struct ProviderExecutionPermit {
    authorization_epoch: vpr_domain::AuthorizationEpoch,
    egress_policy_revision: PolicyRevision,
    cancellation: TurnCancellation,
}

impl ProviderExecutionPermit {
    #[must_use]
    pub const fn authorization_epoch(&self) -> vpr_domain::AuthorizationEpoch {
        self.authorization_epoch
    }

    #[must_use]
    pub const fn egress_policy_revision(&self) -> PolicyRevision {
        self.egress_policy_revision
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    #[must_use]
    pub fn cancellation_probe(&self) -> &dyn CancellationProbe {
        &self.cancellation
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
    egress_policy: EgressPolicyController,
    egress_policy_snapshot: EgressPolicySnapshot,
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
        session: &ActiveSession,
    ) -> Result<Self, RuntimeDenyReason> {
        if session.session.state() != RealtimeSessionState::Active {
            return Err(RuntimeDenyReason::InvalidTurnState);
        }
        if session.session.persona_id() != persona.id() {
            return Err(RuntimeDenyReason::AuthorityDenied);
        }
        let cancellation = TurnCancellation::default();
        let authorization_snapshot = session.authorization.bind_turn(&cancellation)?;
        let egress_policy_snapshot = session.egress_policy.bind_turn(&cancellation)?;
        let turn = Turn::new(turn_id, correlation_id);
        let snapshot = TurnExecutionSnapshot::new(
            turn.id().clone(),
            turn.correlation_id().clone(),
            persona.id().clone(),
            persona.version(),
            persona.mode(),
            authorization_snapshot.epoch(),
            egress_policy_snapshot.revision,
        );
        Ok(Self {
            turn,
            snapshot,
            authorization: session.authorization.clone(),
            authorization_snapshot,
            egress_policy: session.egress_policy.clone(),
            egress_policy_snapshot,
            cancellation,
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
        let authorization_state = self
            .authorization
            .state
            .read()
            .map_err(|_| Rt0ReasonCode::InternalError)?;
        let policy_state = self
            .egress_policy
            .state
            .read()
            .map_err(|_| Rt0ReasonCode::InternalError)?;
        authorization_state
            .authorization
            .validate(self.authorization_snapshot, now_millis)
            .map_err(RuntimeDenyReason::from)
            .map_err(RuntimeDenyReason::reason_code)?;
        if policy_state.revision != self.egress_policy_snapshot.revision {
            return Err(Rt0ReasonCode::EgressDenied);
        }
        if self.cancellation.is_cancelled() {
            return Err(Rt0ReasonCode::TurnCancelled);
        }
        self.turn
            .transition(TurnState::Authorized)
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    /// Enforces the current shared authorization state immediately before provider egress.
    ///
    /// # Errors
    /// Returns `RuntimeDenyReason` if the turn's bound authority is stale/revoked/expired or the
    /// current scope/egress policy forbids the call.
    pub fn begin_external_provider_call(
        &self,
        now_millis: u64,
        authority: &EffectiveAuthority,
        required_scope: &AuthorityScope,
        data_class: DataClass,
    ) -> Result<ProviderExecutionPermit, RuntimeDenyReason> {
        if !matches!(
            self.turn.state(),
            TurnState::Authorized | TurnState::Processing | TurnState::Outputting
        ) {
            return Err(RuntimeDenyReason::InvalidTurnState);
        }
        self.authorization.issue_provider_permit(
            self.authorization_snapshot,
            &ProviderCallContext {
                egress_policy: &self.egress_policy,
                egress_snapshot: self.egress_policy_snapshot,
                cancellation: &self.cancellation,
                now_millis,
                authority,
                required_scope,
                data_class,
            },
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

    /// Terminates the turn as denied and freezes any output evidence.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` when denial is not valid from the current state.
    pub fn deny(&mut self) -> Result<(), Rt0ReasonCode> {
        self.turn
            .transition(TurnState::Denied)
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)?;
        self.freeze_output_segments()?;
        Ok(())
    }

    /// Terminates the turn as failed and freezes any output evidence.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` when failure is not valid from the current state.
    pub fn fail(&mut self) -> Result<(), Rt0ReasonCode> {
        self.turn
            .transition(TurnState::Failed)
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)?;
        self.freeze_output_segments()?;
        Ok(())
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

    fn freeze_output_segments(&mut self) -> Result<(), Rt0ReasonCode> {
        for segment in &mut self.output_segments {
            if !matches!(
                segment.evidence.state(),
                OutputDeliveryState::Cancelled { .. }
            ) {
                segment
                    .evidence
                    .mark_cancelled()
                    .map_err(|_| Rt0ReasonCode::InvalidStateTransition)?;
            }
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
    use vpr_policy::AuthorityLayer;

    fn persona() -> PersonaIdentity {
        PersonaIdentity::new(
            PersonaId::new("persona-1").unwrap(),
            PersonaVersion::new(1).unwrap(),
            PersonaMode::DigitalTwin,
        )
    }

    fn active_session() -> ActiveSession {
        let identity = persona();
        let mut session = ActiveSession::new(
            SessionId::new("session-1").unwrap(),
            identity.id().clone(),
            None,
            true,
            ConsentState::Granted,
            false,
        );
        session.activate().unwrap();
        session
    }

    fn allowed_provider_authority() -> (AuthorityScope, EffectiveAuthority) {
        let scope = AuthorityScope::new("provider.egress").unwrap();
        let authority = EffectiveAuthority::compose(&[AuthorityLayer::new([scope.clone()], [])]);
        (scope, authority)
    }

    fn turn(session: &ActiveSession, name: &str) -> ActiveTurn {
        ActiveTurn::new(
            TurnId::new(format!("turn-{name}")).unwrap(),
            CorrelationId::new(format!("corr-{name}")).unwrap(),
            &persona(),
            session,
        )
        .unwrap()
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
    fn turn_requires_active_matching_session() {
        let identity = persona();
        let session = ActiveSession::new(
            SessionId::new("created").unwrap(),
            identity.id().clone(),
            None,
            true,
            ConsentState::Granted,
            false,
        );
        assert!(matches!(
            ActiveTurn::new(
                TurnId::new("turn-created").unwrap(),
                CorrelationId::new("corr-created").unwrap(),
                &identity,
                &session,
            ),
            Err(RuntimeDenyReason::InvalidTurnState)
        ));
    }

    #[test]
    fn provider_call_before_turn_authorization_is_rejected() {
        let (scope, authority) = allowed_provider_authority();
        let session = active_session();
        let turn = turn(&session, "premature");
        assert!(matches!(
            turn.begin_external_provider_call(0, &authority, &scope, DataClass::Public),
            Err(RuntimeDenyReason::InvalidTurnState)
        ));
    }

    #[test]
    fn session_revoke_cancels_existing_permit_and_denies_new_work() {
        let (scope, authority) = allowed_provider_authority();
        let mut session = active_session();
        let mut turn = turn(&session, "session-revoke");
        turn.authorize(0).unwrap();
        turn.begin_processing().unwrap();
        let permit = turn
            .begin_external_provider_call(0, &authority, &scope, DataClass::Public)
            .unwrap();
        assert!(!permit.is_cancelled());

        session.revoke().unwrap();
        assert_eq!(session.state(), RealtimeSessionState::Revoked);
        assert!(permit.is_cancelled());
        assert!(matches!(
            turn.begin_external_provider_call(1, &authority, &scope, DataClass::Public),
            Err(RuntimeDenyReason::AuthorizationStale)
        ));
    }

    #[test]
    fn local_only_current_policy_blocks_external_provider() {
        let (scope, authority) = allowed_provider_authority();
        let session = active_session();
        session.set_local_only_required(true).unwrap();
        let mut turn = turn(&session, "local-only");
        turn.authorize(0).unwrap();
        assert!(matches!(
            turn.begin_external_provider_call(0, &authority, &scope, DataClass::Public),
            Err(RuntimeDenyReason::LocalOnlyRequired)
        ));
    }

    #[test]
    fn policy_change_cancels_existing_permit_and_stales_bound_turn() {
        let (scope, authority) = allowed_provider_authority();
        let session = active_session();
        let mut turn = turn(&session, "policy-change");
        turn.authorize(0).unwrap();
        turn.begin_processing().unwrap();
        let permit = turn
            .begin_external_provider_call(0, &authority, &scope, DataClass::Biometric)
            .unwrap();
        assert!(!permit.is_cancelled());

        session.set_consent(ConsentState::Revoked).unwrap();
        assert!(permit.is_cancelled());
        assert!(matches!(
            turn.begin_external_provider_call(1, &authority, &scope, DataClass::Biometric),
            Err(RuntimeDenyReason::EgressPolicyStale)
        ));
    }

    #[test]
    fn current_missing_biometric_consent_uses_stable_reason_code() {
        let (scope, authority) = allowed_provider_authority();
        let session = active_session();
        session.set_consent(ConsentState::Missing).unwrap();
        let mut turn = turn(&session, "consent");
        turn.authorize(0).unwrap();
        let denied = turn.begin_external_provider_call(0, &authority, &scope, DataClass::Biometric);
        assert!(matches!(denied, Err(RuntimeDenyReason::ConsentRequired)));
        assert_eq!(
            denied.unwrap_err().reason_code(),
            Rt0ReasonCode::ConsentRequired
        );
    }

    #[test]
    fn authorization_replacement_cancels_permit_and_stales_turn() {
        let (scope, authority) = allowed_provider_authority();
        let session = active_session();
        let mut turn = turn(&session, "auth-replace");
        turn.authorize(0).unwrap();
        let permit = turn
            .begin_external_provider_call(0, &authority, &scope, DataClass::Public)
            .unwrap();
        session.refresh_authorization(None).unwrap();
        assert!(permit.is_cancelled());
        assert!(matches!(
            turn.begin_external_provider_call(1, &authority, &scope, DataClass::Public),
            Err(RuntimeDenyReason::AuthorizationStale)
        ));
    }

    #[test]
    fn permit_and_execution_snapshot_bind_same_revisions() {
        let (scope, authority) = allowed_provider_authority();
        let session = active_session();
        let mut turn = turn(&session, "snapshot");
        turn.authorize(0).unwrap();
        let permit = turn
            .begin_external_provider_call(0, &authority, &scope, DataClass::Public)
            .unwrap();
        assert_eq!(
            permit.authorization_epoch(),
            turn.snapshot().authorization_epoch()
        );
        assert_eq!(
            permit.egress_policy_revision(),
            turn.snapshot().egress_policy_revision()
        );
    }

    #[test]
    fn interruption_preserves_played_prefix_and_unplayed_tail_per_segment() {
        let session = active_session();
        let mut turn = turn(&session, "stream");
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
    fn deny_and_fail_are_real_terminal_paths() {
        let session = active_session();
        let mut denied = turn(&session, "denied");
        denied.authorize(0).unwrap();
        denied.begin_processing().unwrap();
        denied.deny().unwrap();
        assert_eq!(denied.state(), TurnState::Denied);
        assert!(denied.begin_processing().is_err());

        let mut failed = turn(&session, "failed");
        failed.authorize(0).unwrap();
        failed.begin_processing().unwrap();
        failed.begin_output().unwrap();
        let segment = failed.begin_output_segment().unwrap();
        failed.mark_output_sent(segment).unwrap();
        failed.fail().unwrap();
        assert_eq!(failed.state(), TurnState::Failed);
        assert!(matches!(
            failed.output_segments()[0].state(),
            OutputDeliveryState::Cancelled {
                reached: OutputCheckpoint::Sent
            }
        ));
    }

    #[test]
    fn denied_turn_cannot_emit_output_evidence() {
        let session = active_session();
        let mut turn = turn(&session, "denied-output");
        turn.deny().unwrap();
        assert_eq!(
            turn.begin_output_segment(),
            Err(Rt0ReasonCode::InvalidStateTransition)
        );
        assert!(turn.output_segments().is_empty());
    }

    #[test]
    fn invalid_interrupt_does_not_partially_mutate_output_evidence() {
        let session = active_session();
        let mut turn = turn(&session, "terminal");
        turn.deny().unwrap();
        assert_eq!(turn.interrupt(), Err(Rt0ReasonCode::InvalidStateTransition));
        assert!(turn.output_segments().is_empty());
        assert!(!turn.cancellation().is_cancelled());
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
