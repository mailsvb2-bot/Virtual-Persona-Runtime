use std::sync::Arc;

use parking_lot::Mutex;
use vpr_domain::{
    CorrelationId, PersonaIdentity, RealtimeSessionState, Rt0ReasonCode, SessionId, Turn,
    TurnExecutionSnapshot, TurnId, TurnState,
};
use vpr_integration::{
    AudioInput, AvatarPort, GeneratedAudioSink, GeneratedTextSink, GeneratedVideoSink, LlmPort,
    LlmRequest, LlmTextStream, SttPort, SttRequest, Transcript, TtsPort, TtsRequest, UsageEvidence,
};
use vpr_policy::{AuthorityScope, AuthorizationSnapshot, DataClass};

use crate::authority::{
    AuthorizationController, EgressPolicyController, EgressPolicySnapshot, ProviderCallContext,
};
use crate::cancellation::TurnCancellation;
use crate::clock::RuntimeClock;
use crate::error::{ProviderExecutionError, RuntimeDenyReason};
use crate::execution_gate::SessionExecutionGate;
use crate::media_timeline::MediaTimeline;
use crate::output::{OutputSegmentEvidence, OutputSegmentId};
use crate::provider::{ProviderExecutionPermit, ProviderOperation};
use crate::session::ActiveSession;
use crate::turn_state::TurnMutableState;

pub struct AuthorizedLlmStream {
    stream: Box<dyn LlmTextStream>,
    permit: ProviderExecutionPermit,
}

impl AuthorizedLlmStream {
    /// Pulls the next provider text chunk under the exact runtime-issued operation permit.
    ///
    /// # Errors
    /// Returns a typed provider failure or cancellation observed by the runtime permit.
    pub fn next_chunk(&mut self) -> Result<Option<String>, ProviderExecutionError> {
        self.stream
            .next_chunk(&self.permit.cancellation)
            .map_err(ProviderExecutionError::from)
    }

    #[must_use]
    pub fn usage(&self) -> UsageEvidence {
        self.stream.usage()
    }
}

#[derive(Debug, Clone)]
pub struct TurnInterruptHandle {
    state: Arc<Mutex<TurnMutableState>>,
    gate: SessionExecutionGate,
    cancellation: TurnCancellation,
}

impl TurnInterruptHandle {
    /// Interrupts exactly the turn that issued this handle.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` if the turn is already terminal.
    pub fn interrupt(&self) -> Result<(), Rt0ReasonCode> {
        let gate = self.gate.clone();
        let _execution = gate
            .read()
            .map_err(|()| RuntimeDenyReason::InternalError)
            .map_err(RuntimeDenyReason::reason_code)?;
        {
            let mut state = self.state.lock();
            state.interrupt()?;
            self.cancellation.cancel();
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct ActiveTurn {
    pub(crate) state: Arc<Mutex<TurnMutableState>>,
    pub(crate) snapshot: TurnExecutionSnapshot,
    pub(crate) authorization: AuthorizationController,
    pub(crate) authorization_snapshot: AuthorizationSnapshot,
    pub(crate) egress_policy: EgressPolicyController,
    pub(crate) egress_policy_snapshot: EgressPolicySnapshot,
    pub(crate) clock: Arc<dyn RuntimeClock>,
    pub(crate) gate: SessionExecutionGate,
    pub(crate) cancellation: TurnCancellation,
    pub(crate) media_timeline: MediaTimeline,
    pub(crate) session_id: SessionId,
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
        let (authorization_snapshot, egress_policy_snapshot, media_epoch) = {
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
                session.media_timeline.current_epoch(),
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
            state: Arc::new(Mutex::new(TurnMutableState::new(turn, media_epoch))),
            snapshot,
            authorization: session.authorization.clone(),
            authorization_snapshot,
            egress_policy: session.egress_policy.clone(),
            egress_policy_snapshot,
            clock: Arc::clone(&session.clock),
            gate: session.gate.clone(),
            cancellation,
            media_timeline: session.media_timeline.clone(),
            session_id: session.id().clone(),
        })
    }

    #[must_use]
    pub fn state(&self) -> TurnState {
        self.state.lock().state()
    }

    #[must_use]
    pub fn snapshot(&self) -> &TurnExecutionSnapshot {
        &self.snapshot
    }

    #[must_use]
    pub fn output_segments(&self) -> Vec<OutputSegmentEvidence> {
        self.state.lock().output_segments()
    }

    /// Authorizes the turn against the exact authorization revision captured in its snapshot.
    ///
    /// # Errors
    /// Returns a stable authorization reason if the bound revision is no longer current.
    pub fn authorize(&self) -> Result<(), Rt0ReasonCode> {
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
        self.state.lock().transition(TurnState::Authorized)
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
        if !self.state.lock().provider_execution_allowed() {
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

    pub(crate) fn issue_provider_operation(
        &self,
        operation: ProviderOperation,
    ) -> Result<ProviderExecutionPermit, RuntimeDenyReason> {
        let required_scope = operation.required_scope();
        self.issue_provider_permit(&required_scope, operation.data_class())
    }

    /// Opens one pull-based LLM stream through the canonical provider enforcement boundary.
    ///
    /// The returned handle owns the exact provider-operation permit, so cancellation and authority
    /// invalidation remain runtime-controlled while callers decide when generated chunks become
    /// authorized output.
    ///
    /// # Errors
    /// Returns a runtime denial or typed provider failure before a stream handle is exposed.
    pub fn open_llm_stream(
        &self,
        port: &dyn LlmPort,
        request: &LlmRequest,
    ) -> Result<AuthorizedLlmStream, ProviderExecutionError> {
        let permit = self
            .issue_provider_operation(ProviderOperation::Llm)
            .map_err(ProviderExecutionError::from)?;
        let stream = port
            .open_stream(request, &permit.cancellation)
            .map_err(ProviderExecutionError::from)?;
        Ok(AuthorizedLlmStream { stream, permit })
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
        sink: &mut dyn GeneratedTextSink,
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
        request: &SttRequest,
    ) -> Result<(Transcript, UsageEvidence), ProviderExecutionError> {
        let permit = self
            .issue_provider_operation(ProviderOperation::Stt)
            .map_err(ProviderExecutionError::from)?;
        port.transcribe(request, &permit.cancellation)
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
        request: &TtsRequest,
        sink: &mut dyn GeneratedAudioSink,
    ) -> Result<UsageEvidence, ProviderExecutionError> {
        let permit = self
            .issue_provider_operation(ProviderOperation::Tts)
            .map_err(ProviderExecutionError::from)?;
        port.synthesize(request, &permit.cancellation, sink)
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
        sink: &mut dyn GeneratedVideoSink,
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
    pub fn begin_processing(&self) -> Result<(), Rt0ReasonCode> {
        let gate = self.gate.clone();
        let _execution = gate
            .read()
            .map_err(|()| RuntimeDenyReason::InternalError)
            .map_err(RuntimeDenyReason::reason_code)?;
        self.require_execution_active()?;
        self.state.lock().transition(TurnState::Processing)
    }

    /// Moves a processing turn into output delivery.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` for an invalid lifecycle transition.
    pub fn begin_output(&self) -> Result<(), Rt0ReasonCode> {
        let gate = self.gate.clone();
        let _execution = gate
            .read()
            .map_err(|()| RuntimeDenyReason::InternalError)
            .map_err(RuntimeDenyReason::reason_code)?;
        self.require_execution_active()?;
        self.state.lock().transition(TurnState::Outputting)
    }

    /// Completes a turn after output delivery.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` for an invalid lifecycle transition.
    pub fn complete(&self) -> Result<(), Rt0ReasonCode> {
        let gate = self.gate.clone();
        let _execution = gate
            .read()
            .map_err(|()| RuntimeDenyReason::InternalError)
            .map_err(RuntimeDenyReason::reason_code)?;
        self.require_execution_active()?;
        self.state.lock().transition(TurnState::Completed)
    }

    /// Terminates the turn as denied and freezes any output evidence.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` when denial is not valid from the current state.
    pub fn deny(&self) -> Result<(), Rt0ReasonCode> {
        let gate = self.gate.clone();
        let _execution = gate
            .read()
            .map_err(|()| RuntimeDenyReason::InternalError)
            .map_err(RuntimeDenyReason::reason_code)?;
        self.require_not_cancelled()?;
        self.state.lock().transition_and_freeze(TurnState::Denied)
    }

    /// Terminates the turn as failed and freezes any output evidence.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` when failure is not valid from the current state.
    pub fn fail(&self) -> Result<(), Rt0ReasonCode> {
        let gate = self.gate.clone();
        let _execution = gate
            .read()
            .map_err(|()| RuntimeDenyReason::InternalError)
            .map_err(RuntimeDenyReason::reason_code)?;
        self.require_not_cancelled()?;
        self.state.lock().transition_and_freeze(TurnState::Failed)
    }

    /// Opens a generated streamed output segment.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` unless the turn is currently `OUTPUTTING`.
    pub fn begin_output_segment(&self) -> Result<OutputSegmentId, Rt0ReasonCode> {
        let gate = self.gate.clone();
        let _execution = gate
            .read()
            .map_err(|()| RuntimeDenyReason::InternalError)
            .map_err(RuntimeDenyReason::reason_code)?;
        self.require_execution_active()?;
        self.state.lock().begin_output_segment()
    }

    /// Marks one generated segment as sent to transport.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` unless the turn is `OUTPUTTING` and the segment exists
    /// in the expected checkpoint.
    #[cfg(test)]
    pub(crate) fn mark_output_sent(&self, id: OutputSegmentId) -> Result<(), Rt0ReasonCode> {
        let gate = self.gate.clone();
        let _execution = gate
            .read()
            .map_err(|()| RuntimeDenyReason::InternalError)
            .map_err(RuntimeDenyReason::reason_code)?;
        self.require_execution_active()?;
        self.state.lock().mark_output_sent(id)
    }

    /// Marks one sent segment as actually played.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` unless the turn is `OUTPUTTING` and the segment exists
    /// in the expected checkpoint.
    #[cfg(test)]
    pub(crate) fn mark_output_played(&self, id: OutputSegmentId) -> Result<(), Rt0ReasonCode> {
        let gate = self.gate.clone();
        let _execution = gate
            .read()
            .map_err(|()| RuntimeDenyReason::InternalError)
            .map_err(RuntimeDenyReason::reason_code)?;
        self.require_execution_active()?;
        self.state.lock().mark_output_played(id)
    }

    /// Returns a narrow cloneable capability that can only interrupt this turn.
    #[must_use]
    pub fn interrupt_handle(&self) -> TurnInterruptHandle {
        TurnInterruptHandle {
            state: Arc::clone(&self.state),
            gate: self.gate.clone(),
            cancellation: self.cancellation.clone(),
        }
    }

    /// Interrupts the active turn and freezes every streamed segment at its reached checkpoint.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` if the turn is already terminal.
    pub fn interrupt(&self) -> Result<(), Rt0ReasonCode> {
        self.interrupt_handle().interrupt()
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
}
