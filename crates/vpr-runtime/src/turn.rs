use std::sync::Arc;

use vpr_domain::{
    CorrelationId, OutputDeliveryState, OutputEvidence, PersonaIdentity, RealtimeSessionState,
    Rt0ReasonCode, Turn, TurnExecutionSnapshot, TurnId, TurnState,
};
use vpr_integration::{
    AudioInput, AudioSink, AvatarPort, LlmPort, LlmRequest, SttPort, TextSink, Transcript, TtsPort,
    UsageEvidence, VideoSink,
};
use vpr_policy::{AuthorityScope, AuthorizationSnapshot, DataClass};

use crate::authority::{
    AuthorizationController, EgressPolicyController, EgressPolicySnapshot, ProviderCallContext,
};
use crate::cancellation::TurnCancellation;
use crate::clock::RuntimeClock;
use crate::error::{ProviderExecutionError, RuntimeDenyReason};
use crate::execution_gate::SessionExecutionGate;
use crate::output::{OutputSegmentEvidence, OutputSegmentId};
use crate::provider::{ProviderExecutionPermit, ProviderOperation};
use crate::session::ActiveSession;

#[derive(Debug)]
pub struct ActiveTurn {
    pub(crate) turn: Turn,
    pub(crate) snapshot: TurnExecutionSnapshot,
    pub(crate) authorization: AuthorizationController,
    pub(crate) authorization_snapshot: AuthorizationSnapshot,
    pub(crate) egress_policy: EgressPolicyController,
    pub(crate) egress_policy_snapshot: EgressPolicySnapshot,
    pub(crate) clock: Arc<dyn RuntimeClock>,
    pub(crate) gate: SessionExecutionGate,
    pub(crate) cancellation: TurnCancellation,
    pub(crate) output_segments: Vec<OutputSegmentEvidence>,
    pub(crate) next_segment_id: u64,
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
        let cancellation = TurnCancellation::default();
        let (authorization_snapshot, egress_policy_snapshot) = {
            let _execution = session
                .gate
                .read()
                .map_err(|()| RuntimeDenyReason::InternalError)?;
            if session.session.state() != RealtimeSessionState::Active {
                return Err(RuntimeDenyReason::InvalidTurnState);
            }
            if session.session.persona_id() != persona.id() {
                return Err(RuntimeDenyReason::AuthorityDenied);
            }
            (
                session.authorization.bind_turn(&cancellation)?,
                session.egress_policy.bind_turn(&cancellation)?,
            )
        };
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
            clock: Arc::clone(&session.clock),
            gate: session.gate.clone(),
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
    pub fn output_segments(&self) -> &[OutputSegmentEvidence] {
        &self.output_segments
    }

    /// Authorizes the turn against the exact authorization revision captured in its snapshot.
    ///
    /// # Errors
    /// Returns a stable authorization reason if the bound revision is no longer current.
    pub fn authorize(&mut self) -> Result<(), Rt0ReasonCode> {
        let gate = self.gate.clone();
        let _execution = gate
            .read()
            .map_err(|()| RuntimeDenyReason::InternalError)
            .map_err(RuntimeDenyReason::reason_code)?;
        let now_millis = self
            .clock
            .now_millis()
            .ok_or(RuntimeDenyReason::InternalError)
            .map_err(RuntimeDenyReason::reason_code)?;
        self.authorization
            .validate_bound(self.authorization_snapshot, now_millis)
            .map_err(RuntimeDenyReason::reason_code)?;
        self.egress_policy
            .validate_snapshot(self.egress_policy_snapshot)
            .map_err(RuntimeDenyReason::reason_code)?;
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
    pub(crate) fn issue_provider_permit(
        &self,
        required_scope: &AuthorityScope,
        data_class: DataClass,
    ) -> Result<ProviderExecutionPermit, RuntimeDenyReason> {
        let gate = self.gate.clone();
        let _execution = gate.read().map_err(|()| RuntimeDenyReason::InternalError)?;
        if !matches!(
            self.turn.state(),
            TurnState::Authorized | TurnState::Processing | TurnState::Outputting
        ) {
            return Err(RuntimeDenyReason::InvalidTurnState);
        }
        let now_millis = self
            .clock
            .now_millis()
            .ok_or(RuntimeDenyReason::InternalError)?;
        self.authorization.issue_provider_permit(
            self.authorization_snapshot,
            now_millis,
            &ProviderCallContext {
                egress_policy: &self.egress_policy,
                egress_snapshot: self.egress_policy_snapshot,
                cancellation: &self.cancellation,
                clock: &self.clock,
                required_scope,
                data_class,
            },
        )
    }

    fn issue_provider_operation(
        &self,
        operation: ProviderOperation,
    ) -> Result<ProviderExecutionPermit, RuntimeDenyReason> {
        let required_scope = operation.required_scope();
        self.issue_provider_permit(&required_scope, operation.data_class())
    }

    /// Executes one LLM operation through the canonical provider enforcement boundary.
    ///
    /// # Errors
    /// Returns a runtime denial when current authority/policy forbids the operation, or the typed
    /// provider error returned by the adapter.
    pub fn execute_llm(
        &self,
        port: &dyn LlmPort,
        request: &LlmRequest,
        sink: &mut dyn TextSink,
    ) -> Result<UsageEvidence, ProviderExecutionError> {
        let permit = self
            .issue_provider_operation(ProviderOperation::Llm)
            .map_err(ProviderExecutionError::from)?;
        port.stream(request, &permit.cancellation, sink)
            .map_err(ProviderExecutionError::from)
    }

    /// Executes one STT operation through the canonical provider enforcement boundary.
    ///
    /// # Errors
    /// Returns a runtime denial when current authority/policy forbids the operation, or the typed
    /// provider error returned by the adapter.
    pub fn execute_stt(
        &self,
        port: &dyn SttPort,
        input: &AudioInput,
    ) -> Result<(Transcript, UsageEvidence), ProviderExecutionError> {
        let permit = self
            .issue_provider_operation(ProviderOperation::Stt)
            .map_err(ProviderExecutionError::from)?;
        port.transcribe(input, &permit.cancellation)
            .map_err(ProviderExecutionError::from)
    }

    /// Executes one TTS operation through the canonical provider enforcement boundary.
    ///
    /// # Errors
    /// Returns a runtime denial when current authority/policy forbids the operation, or the typed
    /// provider error returned by the adapter.
    pub fn execute_tts(
        &self,
        port: &dyn TtsPort,
        text: &str,
        sink: &mut dyn AudioSink,
    ) -> Result<UsageEvidence, ProviderExecutionError> {
        let permit = self
            .issue_provider_operation(ProviderOperation::Tts)
            .map_err(ProviderExecutionError::from)?;
        port.synthesize(text, &permit.cancellation, sink)
            .map_err(ProviderExecutionError::from)
    }

    /// Executes one avatar-render operation through the canonical provider enforcement boundary.
    ///
    /// # Errors
    /// Returns a runtime denial when current authority/policy forbids the operation, or the typed
    /// provider error returned by the adapter.
    pub fn execute_avatar(
        &self,
        port: &dyn AvatarPort,
        audio: &AudioInput,
        sink: &mut dyn VideoSink,
    ) -> Result<UsageEvidence, ProviderExecutionError> {
        let permit = self
            .issue_provider_operation(ProviderOperation::Avatar)
            .map_err(ProviderExecutionError::from)?;
        port.render(audio, &permit.cancellation, sink)
            .map_err(ProviderExecutionError::from)
    }

    /// Moves an authorized turn into processing.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` for an invalid lifecycle transition.
    pub fn begin_processing(&mut self) -> Result<(), Rt0ReasonCode> {
        let gate = self.gate.clone();
        let _execution = gate
            .read()
            .map_err(|()| RuntimeDenyReason::InternalError)
            .map_err(RuntimeDenyReason::reason_code)?;
        self.require_execution_active()?;
        self.turn
            .transition(TurnState::Processing)
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    /// Moves a processing turn into output delivery.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` for an invalid lifecycle transition.
    pub fn begin_output(&mut self) -> Result<(), Rt0ReasonCode> {
        let gate = self.gate.clone();
        let _execution = gate
            .read()
            .map_err(|()| RuntimeDenyReason::InternalError)
            .map_err(RuntimeDenyReason::reason_code)?;
        self.require_execution_active()?;
        self.turn
            .transition(TurnState::Outputting)
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    /// Completes a turn after output delivery.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` for an invalid lifecycle transition.
    pub fn complete(&mut self) -> Result<(), Rt0ReasonCode> {
        let gate = self.gate.clone();
        let _execution = gate
            .read()
            .map_err(|()| RuntimeDenyReason::InternalError)
            .map_err(RuntimeDenyReason::reason_code)?;
        self.require_execution_active()?;
        self.turn
            .transition(TurnState::Completed)
            .map_err(|_| Rt0ReasonCode::InvalidStateTransition)
    }

    /// Terminates the turn as denied and freezes any output evidence.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` when denial is not valid from the current state.
    pub fn deny(&mut self) -> Result<(), Rt0ReasonCode> {
        let gate = self.gate.clone();
        let _execution = gate
            .read()
            .map_err(|()| RuntimeDenyReason::InternalError)
            .map_err(RuntimeDenyReason::reason_code)?;
        self.require_not_cancelled()?;
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
        let gate = self.gate.clone();
        let _execution = gate
            .read()
            .map_err(|()| RuntimeDenyReason::InternalError)
            .map_err(RuntimeDenyReason::reason_code)?;
        self.require_not_cancelled()?;
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
        let gate = self.gate.clone();
        let _execution = gate
            .read()
            .map_err(|()| RuntimeDenyReason::InternalError)
            .map_err(RuntimeDenyReason::reason_code)?;
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
        let gate = self.gate.clone();
        let _execution = gate
            .read()
            .map_err(|()| RuntimeDenyReason::InternalError)
            .map_err(RuntimeDenyReason::reason_code)?;
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
        let gate = self.gate.clone();
        let _execution = gate
            .read()
            .map_err(|()| RuntimeDenyReason::InternalError)
            .map_err(RuntimeDenyReason::reason_code)?;
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
        let gate = self.gate.clone();
        let _execution = gate
            .read()
            .map_err(|()| RuntimeDenyReason::InternalError)
            .map_err(RuntimeDenyReason::reason_code)?;
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

    fn require_execution_active(&self) -> Result<(), Rt0ReasonCode> {
        self.require_not_cancelled()?;
        let now_millis = self
            .clock
            .now_millis()
            .ok_or(Rt0ReasonCode::InternalError)?;
        self.authorization
            .validate_bound(self.authorization_snapshot, now_millis)
            .map_err(RuntimeDenyReason::reason_code)?;
        self.egress_policy
            .validate_snapshot(self.egress_policy_snapshot)
            .map_err(RuntimeDenyReason::reason_code)
    }

    fn require_not_cancelled(&self) -> Result<(), Rt0ReasonCode> {
        if self.cancellation.is_cancelled() {
            Err(Rt0ReasonCode::TurnCancelled)
        } else {
            Ok(())
        }
    }

    fn require_outputting(&self) -> Result<(), Rt0ReasonCode> {
        self.require_execution_active()?;
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
