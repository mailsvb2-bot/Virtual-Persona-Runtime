use vpr_domain::{CorrelationId, SessionId, TurnId};

use crate::{CancellationProbe, PcmSampleFormat};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportErrorKind {
    Unavailable,
    Cancelled,
    DeliveryUncertain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportError {
    pub kind: TransportErrorKind,
    pub retryable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaTimelineStamp {
    pub epoch: u64,
    pub sequence: u64,
    pub presentation_time_micros: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct RealtimeTextOutputEvent {
    pub turn_id: TurnId,
    pub correlation_id: CorrelationId,
    pub sequence: u64,
    pub text: String,
}

impl std::fmt::Debug for RealtimeTextOutputEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RealtimeTextOutputEvent")
            .field("turn_id", &self.turn_id)
            .field("correlation_id", &self.correlation_id)
            .field("sequence", &self.sequence)
            .field("text_chars", &self.text.chars().count())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RealtimeAudioOutputEvent {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub correlation_id: CorrelationId,
    pub timeline: MediaTimelineStamp,
    pub pcm: Vec<u8>,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub sample_format: PcmSampleFormat,
}

impl std::fmt::Debug for RealtimeAudioOutputEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RealtimeAudioOutputEvent")
            .field("session_id", &self.session_id)
            .field("turn_id", &self.turn_id)
            .field("correlation_id", &self.correlation_id)
            .field("timeline", &self.timeline)
            .field("pcm_bytes", &self.pcm.len())
            .field("sample_rate_hz", &self.sample_rate_hz)
            .field("channels", &self.channels)
            .field("sample_format", &self.sample_format)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RealtimeVideoOutputEvent {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub correlation_id: CorrelationId,
    pub timeline: MediaTimelineStamp,
    pub encoded_frame: Vec<u8>,
}

impl std::fmt::Debug for RealtimeVideoOutputEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RealtimeVideoOutputEvent")
            .field("session_id", &self.session_id)
            .field("turn_id", &self.turn_id)
            .field("correlation_id", &self.correlation_id)
            .field("timeline", &self.timeline)
            .field("encoded_bytes", &self.encoded_frame.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealtimeMediaFlushEvent {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub correlation_id: CorrelationId,
    pub epoch: u64,
    pub through_sequence: Option<u64>,
}

pub trait RealtimeOutputPort: Send + Sync {
    /// Sends one canonical text event to the participant transport.
    ///
    /// Returning `Ok(())` confirms transport send, not participant playback.
    ///
    /// # Errors
    /// Returns a typed transport failure. `DeliveryUncertain` forbids blind retry.
    fn send_text(
        &self,
        event: &RealtimeTextOutputEvent,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), TransportError>;

    /// Sends one canonical PCM audio packet.
    ///
    /// # Errors
    /// The default is an explicit unsupported/unavailable result for text-only transports.
    fn send_audio(
        &self,
        _event: &RealtimeAudioOutputEvent,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<(), TransportError> {
        Err(media_unavailable())
    }

    /// Sends one canonical video frame.
    ///
    /// # Errors
    /// The default is an explicit unsupported/unavailable result for text-only transports.
    fn send_video(
        &self,
        _event: &RealtimeVideoOutputEvent,
        _cancellation: &dyn CancellationProbe,
    ) -> Result<(), TransportError> {
        Err(media_unavailable())
    }

    /// Flushes queued media for one interrupted turn through the supplied canonical sequence.
    ///
    /// # Errors
    /// The default is an explicit unsupported/unavailable result for text-only transports.
    fn flush_media(&self, _event: &RealtimeMediaFlushEvent) -> Result<(), TransportError> {
        Err(media_unavailable())
    }
}

const fn media_unavailable() -> TransportError {
    TransportError {
        kind: TransportErrorKind::Unavailable,
        retryable: false,
    }
}
