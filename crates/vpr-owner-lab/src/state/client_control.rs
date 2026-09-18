use serde::Serialize;
use vpr_integration::{RealtimeAvatarClientCommand, RealtimeAvatarClientEvent};

use super::{LabError, OwnerLabEngine, map_provider_execution};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LabClientControl {
    pub data_channel_label: String,
    pub interrupt: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LabClientEvent {
    PlaybackStarted { playback_id: String },
    PlaybackDone,
}

#[derive(Clone, Serialize, PartialEq, Eq)]
pub struct LabClientCommand {
    pub data_channel_label: String,
    pub payload: String,
}

impl std::fmt::Debug for LabClientCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LabClientCommand")
            .field("data_channel_label", &self.data_channel_label)
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
        playback_id: &str,
    ) -> Result<LabClientCommand, LabError> {
        if playback_id.trim().is_empty() {
            return Err(LabError::InvalidInput);
        }
        let turn = self.new_turn()?;
        let handle = self.avatar.as_ref().ok_or(LabError::InvalidState)?;
        let command: RealtimeAvatarClientCommand = turn
            .prepare_realtime_avatar_client_interrupt(self.provider.as_ref(), handle, playback_id)
            .map_err(map_provider_execution)?;
        Ok(LabClientCommand {
            data_channel_label: command.data_channel_label,
            payload: command.payload,
        })
    }
}
