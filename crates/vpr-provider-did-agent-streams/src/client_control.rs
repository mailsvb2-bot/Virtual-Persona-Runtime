use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use vpr_integration::{
    ProviderError, ProviderErrorKind, RealtimeAvatarClientCommand, RealtimeAvatarClientControl,
    RealtimeAvatarClientEvent, RealtimeAvatarSession,
};

use super::invalid_response;

const DID_DATA_CHANNEL_LABEL: &str = "JanusDataChannel";
const MAX_PLAYBACK_ID_BYTES: usize = 512;
const MAX_CLIENT_EVENT_BYTES: usize = 64 * 1024;

#[derive(Default)]
pub(super) struct DidClientControlRegistry {
    interrupt_sessions: Mutex<HashSet<(String, String)>>,
}

impl DidClientControlRegistry {
    pub(super) fn register_interrupt(
        &self,
        session: &RealtimeAvatarSession,
    ) -> Result<(), ProviderError> {
        self.interrupt_sessions
            .lock()
            .map_err(|_| invalid_response())?
            .insert(session_key(session));
        Ok(())
    }

    pub(super) fn control(
        &self,
        session: &RealtimeAvatarSession,
    ) -> Option<RealtimeAvatarClientControl> {
        self.interrupt_sessions
            .lock()
            .ok()?
            .contains(&session_key(session))
            .then(|| RealtimeAvatarClientControl {
                data_channel_label: DID_DATA_CHANNEL_LABEL.to_owned(),
                interrupt: true,
            })
    }

    pub(super) fn parse_event(
        &self,
        message: &str,
    ) -> Result<Option<RealtimeAvatarClientEvent>, ProviderError> {
        if message.len() > MAX_CLIENT_EVENT_BYTES {
            return Err(invalid_response());
        }
        let Some((subject, raw_payload)) = message.split_once(':') else {
            return Ok(None);
        };
        match subject {
            "stream/started" => {
                let payload: StreamStartedPayload =
                    serde_json::from_str(raw_payload).map_err(|_| invalid_response())?;
                let playback_id = payload
                    .metadata
                    .and_then(|metadata| metadata.video_id)
                    .ok_or_else(invalid_response)?;
                if playback_id.trim().is_empty() || playback_id.len() > MAX_PLAYBACK_ID_BYTES {
                    return Err(invalid_response());
                }
                Ok(Some(RealtimeAvatarClientEvent::PlaybackStarted {
                    playback_id,
                }))
            }
            "stream/done" => Ok(Some(RealtimeAvatarClientEvent::PlaybackDone)),
            _ => Ok(None),
        }
    }

    pub(super) fn prepare_interrupt(
        &self,
        session: &RealtimeAvatarSession,
        playback_id: &str,
    ) -> Result<RealtimeAvatarClientCommand, ProviderError> {
        let playback_id = playback_id.trim();
        if playback_id.is_empty() || playback_id.len() > MAX_PLAYBACK_ID_BYTES {
            return Err(invalid_response());
        }
        if !self
            .interrupt_sessions
            .lock()
            .map_err(|_| invalid_response())?
            .contains(&session_key(session))
        {
            return Err(ProviderError {
                kind: ProviderErrorKind::Unavailable,
                retryable: false,
            });
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid_response())?
            .as_millis();
        let timestamp = u64::try_from(timestamp).map_err(|_| invalid_response())?;
        let payload = serde_json::to_string(&StreamInterruptPayload {
            kind: "stream/interrupt",
            video_id: playback_id,
            timestamp,
        })
        .map_err(|_| invalid_response())?;
        Ok(RealtimeAvatarClientCommand {
            data_channel_label: DID_DATA_CHANNEL_LABEL.to_owned(),
            payload,
        })
    }

    pub(super) fn forget(&self, session: &RealtimeAvatarSession) -> Result<(), ProviderError> {
        self.interrupt_sessions
            .lock()
            .map_err(|_| invalid_response())?
            .remove(&session_key(session));
        Ok(())
    }
}

fn session_key(session: &RealtimeAvatarSession) -> (String, String) {
    (
        session.provider_stream_id.clone(),
        session.provider_session_id.clone(),
    )
}

#[derive(Deserialize)]
struct StreamStartedPayload {
    metadata: Option<StreamStartedMetadata>,
}

#[derive(Deserialize)]
struct StreamStartedMetadata {
    #[serde(rename = "videoId")]
    video_id: Option<String>,
}

#[derive(Serialize)]
struct StreamInterruptPayload<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(rename = "videoId")]
    video_id: &'a str,
    timestamp: u64,
}
