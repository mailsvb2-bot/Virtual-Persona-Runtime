use std::sync::Arc;

use vpr_domain::{PersonaId, RealtimeSession, RealtimeSessionState, Rt0ReasonCode, SessionId};
use vpr_policy::ConsentState;

use crate::authority::{AuthorizationController, EgressPolicyController};
use crate::clock::{RuntimeClock, SystemClock};
use crate::error::RuntimeDenyReason;

#[derive(Debug)]
pub struct ActiveSession {
    pub(crate) session: RealtimeSession,
    pub(crate) authorization: AuthorizationController,
    pub(crate) egress_policy: EgressPolicyController,
    pub(crate) clock: Arc<dyn RuntimeClock>,
}

impl ActiveSession {
    #[must_use]
    pub fn new(
        id: SessionId,
        persona_id: PersonaId,
        expires_at_millis: Option<u64>,
        provider_policy_allows: bool,
        consent: ConsentState,
        local_only_required: bool,
    ) -> Self {
        Self::with_clock(
            id,
            persona_id,
            expires_at_millis,
            provider_policy_allows,
            consent,
            local_only_required,
            Arc::new(SystemClock),
        )
    }

    pub(crate) fn with_clock(
        id: SessionId,
        persona_id: PersonaId,
        expires_at_millis: Option<u64>,
        provider_policy_allows: bool,
        consent: ConsentState,
        local_only_required: bool,
        clock: Arc<dyn RuntimeClock>,
    ) -> Self {
        Self {
            session: RealtimeSession::new(id, persona_id),
            authorization: AuthorizationController::new(expires_at_millis),
            egress_policy: EgressPolicyController::new(
                provider_policy_allows,
                consent,
                local_only_required,
            ),
            clock,
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
