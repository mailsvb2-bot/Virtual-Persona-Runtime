use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::{
    AuthorizationEpoch, CorrelationId, PersonaId, PersonaMode, PersonaVersion, SessionId, TurnId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealtimeSessionState {
    Created,
    Active,
    Draining,
    Revoked,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealtimeSession {
    id: SessionId,
    persona_id: PersonaId,
    state: RealtimeSessionState,
}

impl RealtimeSession {
    #[must_use]
    pub fn new(id: SessionId, persona_id: PersonaId) -> Self {
        Self {
            id,
            persona_id,
            state: RealtimeSessionState::Created,
        }
    }

    #[must_use]
    pub fn id(&self) -> &SessionId {
        &self.id
    }

    #[must_use]
    pub fn persona_id(&self) -> &PersonaId {
        &self.persona_id
    }

    #[must_use]
    pub fn state(&self) -> RealtimeSessionState {
        self.state
    }

    /// Advances the canonical realtime-session lifecycle.
    ///
    /// # Errors
    /// Returns `SessionTransitionError` when the transition is forbidden.
    pub fn transition(&mut self, next: RealtimeSessionState) -> Result<(), SessionTransitionError> {
        let valid = matches!(
            (self.state, next),
            (
                RealtimeSessionState::Created,
                RealtimeSessionState::Active | RealtimeSessionState::Revoked
            ) | (
                RealtimeSessionState::Active,
                RealtimeSessionState::Draining | RealtimeSessionState::Revoked
            ) | (
                RealtimeSessionState::Draining,
                RealtimeSessionState::Closed | RealtimeSessionState::Revoked
            ) | (RealtimeSessionState::Revoked, RealtimeSessionState::Closed)
        );
        if !valid {
            return Err(SessionTransitionError {
                from: self.state,
                to: next,
            });
        }
        self.state = next;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionTransitionError {
    pub from: RealtimeSessionState,
    pub to: RealtimeSessionState,
}

impl Display for SessionTransitionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "invalid session transition: {:?} -> {:?}",
            self.from, self.to
        )
    }
}

impl Error for SessionTransitionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnExecutionSnapshot {
    turn_id: TurnId,
    correlation_id: CorrelationId,
    persona_id: PersonaId,
    persona_version: PersonaVersion,
    persona_mode: PersonaMode,
    authorization_epoch: AuthorizationEpoch,
}

impl TurnExecutionSnapshot {
    #[must_use]
    pub const fn new(
        turn_id: TurnId,
        correlation_id: CorrelationId,
        persona_id: PersonaId,
        persona_version: PersonaVersion,
        persona_mode: PersonaMode,
        authorization_epoch: AuthorizationEpoch,
    ) -> Self {
        Self {
            turn_id,
            correlation_id,
            persona_id,
            persona_version,
            persona_mode,
            authorization_epoch,
        }
    }

    #[must_use]
    pub fn turn_id(&self) -> &TurnId {
        &self.turn_id
    }

    #[must_use]
    pub fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }

    #[must_use]
    pub fn persona_id(&self) -> &PersonaId {
        &self.persona_id
    }

    #[must_use]
    pub const fn persona_version(&self) -> PersonaVersion {
        self.persona_version
    }

    #[must_use]
    pub const fn persona_mode(&self) -> PersonaMode {
        self.persona_mode
    }

    #[must_use]
    pub const fn authorization_epoch(&self) -> AuthorizationEpoch {
        self.authorization_epoch
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnState {
    Received,
    Authorized,
    Processing,
    Outputting,
    Completed,
    Denied,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    id: TurnId,
    correlation_id: CorrelationId,
    state: TurnState,
}

impl Turn {
    #[must_use]
    pub fn new(id: TurnId, correlation_id: CorrelationId) -> Self {
        Self {
            id,
            correlation_id,
            state: TurnState::Received,
        }
    }

    #[must_use]
    pub fn id(&self) -> &TurnId {
        &self.id
    }

    #[must_use]
    pub fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }

    #[must_use]
    pub fn state(&self) -> TurnState {
        self.state
    }

    /// Advances the canonical turn lifecycle.
    ///
    /// # Errors
    /// Returns `TurnTransitionError` when the requested transition is not allowed.
    pub fn transition(&mut self, next: TurnState) -> Result<(), TurnTransitionError> {
        let valid = matches!(
            (self.state, next),
            (
                TurnState::Received,
                TurnState::Authorized
                    | TurnState::Denied
                    | TurnState::Failed
                    | TurnState::Cancelled
            ) | (
                TurnState::Authorized,
                TurnState::Processing | TurnState::Failed | TurnState::Cancelled
            ) | (
                TurnState::Processing,
                TurnState::Outputting | TurnState::Failed | TurnState::Cancelled
            ) | (
                TurnState::Outputting,
                TurnState::Completed | TurnState::Failed | TurnState::Cancelled
            )
        );
        if !valid {
            return Err(TurnTransitionError {
                from: self.state,
                to: next,
            });
        }
        self.state = next;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnTransitionError {
    pub from: TurnState,
    pub to: TurnState,
}

impl Display for TurnTransitionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "invalid turn transition: {:?} -> {:?}",
            self.from, self.to
        )
    }
}

impl Error for TurnTransitionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputCheckpoint {
    Pending,
    Generated,
    Sent,
    Played,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputDeliveryState {
    Pending,
    Generated,
    Sent,
    Played,
    Cancelled { reached: OutputCheckpoint },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputEvidence {
    state: OutputDeliveryState,
}

impl Default for OutputEvidence {
    fn default() -> Self {
        Self {
            state: OutputDeliveryState::Pending,
        }
    }
}

impl OutputEvidence {
    #[must_use]
    pub fn state(&self) -> OutputDeliveryState {
        self.state
    }

    /// Marks generation as having begun/completed for this evidence segment.
    ///
    /// # Errors
    /// Returns `OutputTransitionError` unless the segment is still pending.
    pub fn mark_generated(&mut self) -> Result<(), OutputTransitionError> {
        self.transition(OutputDeliveryState::Generated)
    }

    /// Marks the generated segment as sent to transport.
    ///
    /// # Errors
    /// Returns `OutputTransitionError` unless generation was recorded first.
    pub fn mark_sent(&mut self) -> Result<(), OutputTransitionError> {
        self.transition(OutputDeliveryState::Sent)
    }

    /// Marks the sent segment as actually played to the participant.
    ///
    /// # Errors
    /// Returns `OutputTransitionError` unless transport send was recorded first.
    pub fn mark_played(&mut self) -> Result<(), OutputTransitionError> {
        self.transition(OutputDeliveryState::Played)
    }

    /// Cancels the segment while retaining the furthest reached delivery checkpoint.
    ///
    /// # Errors
    /// Returns `OutputTransitionError` if the segment was already cancelled.
    pub fn mark_cancelled(&mut self) -> Result<(), OutputTransitionError> {
        let reached = match self.state {
            OutputDeliveryState::Pending => OutputCheckpoint::Pending,
            OutputDeliveryState::Generated => OutputCheckpoint::Generated,
            OutputDeliveryState::Sent => OutputCheckpoint::Sent,
            OutputDeliveryState::Played => OutputCheckpoint::Played,
            OutputDeliveryState::Cancelled { .. } => {
                return Err(OutputTransitionError {
                    from: self.state,
                    to: self.state,
                });
            }
        };
        self.state = OutputDeliveryState::Cancelled { reached };
        Ok(())
    }

    fn transition(&mut self, next: OutputDeliveryState) -> Result<(), OutputTransitionError> {
        let valid = matches!(
            (self.state, next),
            (OutputDeliveryState::Pending, OutputDeliveryState::Generated)
                | (OutputDeliveryState::Generated, OutputDeliveryState::Sent)
                | (OutputDeliveryState::Sent, OutputDeliveryState::Played)
        );
        if !valid {
            return Err(OutputTransitionError {
                from: self.state,
                to: next,
            });
        }
        self.state = next;
        Ok(())
    }

    #[must_use]
    pub fn eligible_as_spoken(&self) -> bool {
        matches!(
            self.state,
            OutputDeliveryState::Played
                | OutputDeliveryState::Cancelled {
                    reached: OutputCheckpoint::Played
                }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputTransitionError {
    pub from: OutputDeliveryState,
    pub to: OutputDeliveryState,
}

impl Display for OutputTransitionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "invalid output transition: {:?} -> {:?}",
            self.from, self.to
        )
    }
}

impl Error for OutputTransitionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_unplayed_output_is_not_spoken() {
        let mut output = OutputEvidence::default();
        output.mark_generated().unwrap();
        output.mark_sent().unwrap();
        output.mark_cancelled().unwrap();
        assert!(!output.eligible_as_spoken());
    }

    #[test]
    fn revoked_session_can_only_close() {
        let mut session = RealtimeSession::new(
            SessionId::new("session-1").unwrap(),
            crate::PersonaId::new("persona-1").unwrap(),
        );
        session.transition(RealtimeSessionState::Active).unwrap();
        assert!(session.transition(RealtimeSessionState::Closed).is_err());
        session.transition(RealtimeSessionState::Revoked).unwrap();
        assert!(session.transition(RealtimeSessionState::Active).is_err());
        session.transition(RealtimeSessionState::Closed).unwrap();
    }

    #[test]
    fn terminal_turn_cannot_be_reanimated() {
        let mut turn = Turn::new(
            TurnId::new("turn-1").unwrap(),
            CorrelationId::new("corr-1").unwrap(),
        );
        turn.transition(TurnState::Cancelled).unwrap();
        assert!(turn.transition(TurnState::Authorized).is_err());
    }
}
