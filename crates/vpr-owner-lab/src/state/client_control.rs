use serde::Serialize;
use vpr_integration::{
    RealtimeAvatarClientCommand, RealtimeAvatarClientControl, RealtimeAvatarClientEvent,
    RealtimeAvatarClientRoute,
};

use super::{LabError, OwnerLabEngine, map_provider_execution};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LabClientControl {
    pub event_route: Option<LabClientRoute>,
    pub interrupt: bool,
    pub interrupt_requires_playback_id: bool,
    pub text_input: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LabClientRoute {
    WebRtcDataChannel { label: String },
    LiveKitTextTopic { topic: String },
}

impl From<&RealtimeAvatarClientControl> for LabClientControl {
    fn from(value: &RealtimeAvatarClientControl) -> Self {
        Self {
            event_route: value.event_route.as_ref().map(LabClientRoute::from),
            interrupt: value.interrupt,
            interrupt_requires_playback_id: value.interrupt_requires_playback_id,
            text_input: value.text_input,
        }
    }
}

impl From<&RealtimeAvatarClientRoute> for LabClientRoute {
    fn from(value: &RealtimeAvatarClientRoute) -> Self {
        match value {
            RealtimeAvatarClientRoute::WebRtcDataChannel { label } => {
                Self::WebRtcDataChannel {
                    label: label.clone(),
                }
            }
            RealtimeAvatarClientRoute::LiveKitTextTopic { topic } => Self::LiveKitTextTopic {
                topic: topic.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LabClientEvent {
    PlaybackStarted { playback_id: String },
    PlaybackDone,
}

#[derive(Clone, Serialize, PartialEq, Eq)]
pub struct LabClientCommand {
    pub route: LabClientRoute,
    pub payload: String,
}

impl From<RealtimeAvatarClientCommand> for LabClientCommand {
    fn from(value: RealtimeAvatarClientCommand) -> Self {
        Self {
            route: LabClientRoute::from(&value.route),
            payload: value.payload,
        }
    }
}

impl std::fmt::Debug for LabClientCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LabClientCommand")
            .field("route", &self.route)
            .field("payload_bytes", &self.payload.len())
            .finish()
    }
}

impl OwnerLabEngine {
    /// Normalizes one transient browser-received provider data-channel message against the exact
    /// current avatar session. Raw provider payloads are not retained.
    ///
    /// # Errors
    /// Fails closed for stale sessions or malformed recognized provider events.
    pub fn parse_client_event(
        &mut self,
        message: &str,
    ) -> Result<Option<LabClientEvent>, LabError> {
        if message.is_empty() {
            return Err(LabError::InvalidInput);
        }
        let turn = self.new_turn()?;
        let handle = self.avatar.as_ref().ok_or(LabError::InvalidState)?;
        let event = turn
            .parse_realtime_avatar_client_event(self.provider.as_ref(), handle, message)
            .map_err(map_provider_execution)?;
        Ok(event.map(|event| match event {
            RealtimeAvatarClientEvent::PlaybackStarted { playback_id } => {
                LabClientEvent::PlaybackStarted { playback_id }
            }
            RealtimeAvatarClientEvent::PlaybackDone => LabClientEvent::PlaybackDone,
        }))
    }

    /// Prepares one provider-specific browser data-channel interruption command through the
    /// canonical runtime authority boundary.
    ///
    /// # Errors
    /// Fails closed when no session-scoped client interruption is available, the playback
    /// identifier is malformed, or current authority/egress policy denies the operation.
    pub fn prepare_client_interrupt(
        &mut self,
        playback_id: Option<&str>,
    ) -> Result<LabClientCommand, LabError> {
        let playback_id = playback_id
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let turn = self.new_turn()?;
        let handle = self.avatar.as_ref().ok_or(LabError::InvalidState)?;
        let control = handle.client_control().ok_or(LabError::InvalidState)?;
        if control.interrupt_requires_playback_id && playback_id.is_none() {
            return Err(LabError::InvalidInput);
        }
        let command = turn
            .prepare_realtime_avatar_client_interrupt(
                self.provider.as_ref(),
                handle,
                playback_id,
            )
            .map_err(map_provider_execution)?;
        Ok(command.into())
    }
}
