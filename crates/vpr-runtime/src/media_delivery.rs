use vpr_domain::Rt0ReasonCode;
use vpr_integration::{
    GeneratedAudioBuffer, GeneratedVideoFrame, PcmSampleFormat, RealtimeAudioOutputEvent,
    RealtimeMediaFlushEvent, RealtimeOutputPort, RealtimeVideoOutputEvent,
};

use crate::{ActiveTurn, OutputDeliveryError, OutputDeliveryHandle};

impl ActiveTurn {
    /// Delivers one generated PCM packet on the canonical session media timeline.
    ///
    /// Audio packets advance a turn-local cursor by their exact PCM duration. The transport only
    /// sees runtime-normalized epoch/sequence/presentation timestamps.
    ///
    /// # Errors
    /// Returns a lifecycle/timeline error before transport on malformed audio or stale ordering,
    /// otherwise the same typed delivery result used by text output.
    pub fn deliver_audio(
        &self,
        port: &dyn RealtimeOutputPort,
        audio: &GeneratedAudioBuffer,
    ) -> Result<OutputDeliveryHandle, OutputDeliveryError> {
        let (sample_rate_hz, channels, sample_format, duration_micros) = audio_contract(audio)
            .ok_or(OutputDeliveryError::Runtime(Rt0ReasonCode::InternalError))?;
        let now_millis = self
            .clock
            .now_millis()
            .ok_or(OutputDeliveryError::Runtime(Rt0ReasonCode::InternalError))?;
        let timeline = self.state.lock().next_audio_media_stamp(
            &self.media_timeline,
            now_millis,
            duration_micros,
        )?;
        let segment_id = self.begin_output_segment()?;
        let event = RealtimeAudioOutputEvent {
            session_id: self.media_timeline.session_id().clone(),
            turn_id: self.snapshot.turn_id().clone(),
            correlation_id: self.snapshot.correlation_id().clone(),
            timeline,
            pcm: audio.pcm().to_vec(),
            sample_rate_hz,
            channels,
            sample_format,
        };
        self.reconcile_transport_send(segment_id, port.send_audio(&event, &self.cancellation))
    }

    /// Delivers one generated video frame after mapping provider-relative timestamps to the
    /// canonical session timeline. The first provider timestamp becomes offset zero.
    ///
    /// # Errors
    /// Returns before transport for empty frames, decreasing provider timestamps, stale timeline
    /// epochs, or invalid turn lifecycle.
    pub fn deliver_video_frame(
        &self,
        port: &dyn RealtimeOutputPort,
        frame: &GeneratedVideoFrame,
    ) -> Result<OutputDeliveryHandle, OutputDeliveryError> {
        if frame.encoded_frame.is_empty() {
            return Err(OutputDeliveryError::Runtime(Rt0ReasonCode::InternalError));
        }
        let now_millis = self
            .clock
            .now_millis()
            .ok_or(OutputDeliveryError::Runtime(Rt0ReasonCode::InternalError))?;
        let timeline = self.state.lock().next_video_media_stamp(
            &self.media_timeline,
            now_millis,
            frame.timestamp_micros,
        )?;
        let segment_id = self.begin_output_segment()?;
        let event = RealtimeVideoOutputEvent {
            session_id: self.media_timeline.session_id().clone(),
            turn_id: self.snapshot.turn_id().clone(),
            correlation_id: self.snapshot.correlation_id().clone(),
            timeline,
            encoded_frame: frame.encoded_frame.clone(),
        };
        self.reconcile_transport_send(segment_id, port.send_video(&event, &self.cancellation))
    }

    /// Cancels the turn and flushes every queued media event allocated through its last sequence.
    ///
    /// The turn remains cancelled even when the transport reports a flush failure.
    ///
    /// # Errors
    /// Returns a lifecycle error if interruption is invalid, or the typed transport flush error.
    pub fn interrupt_and_flush_media(
        &self,
        port: &dyn RealtimeOutputPort,
    ) -> Result<(), OutputDeliveryError> {
        self.interrupt()?;
        let (epoch, through_sequence) = self.state.lock().media_flush_context();
        let event = RealtimeMediaFlushEvent {
            session_id: self.media_timeline.session_id().clone(),
            turn_id: self.snapshot.turn_id().clone(),
            correlation_id: self.snapshot.correlation_id().clone(),
            epoch,
            through_sequence,
        };
        port.flush_media(&event)
            .map_err(OutputDeliveryError::Transport)
    }
}

fn audio_contract(audio: &GeneratedAudioBuffer) -> Option<(u32, u16, PcmSampleFormat, u64)> {
    let sample_rate_hz = audio.sample_rate_hz()?;
    let channels = audio.channels()?;
    let sample_format = audio.sample_format()?;
    if audio.pcm().is_empty() || sample_rate_hz == 0 || channels == 0 {
        return None;
    }
    let bytes_per_sample = match sample_format {
        PcmSampleFormat::S16Le => 2_u64,
    };
    let frame_bytes = u64::from(channels).checked_mul(bytes_per_sample)?;
    let pcm_len = u64::try_from(audio.pcm().len()).ok()?;
    if pcm_len % frame_bytes != 0 {
        return None;
    }
    let frames = pcm_len / frame_bytes;
    let duration_micros = frames
        .checked_mul(1_000_000)?
        .checked_div(u64::from(sample_rate_hz))?;
    Some((sample_rate_hz, channels, sample_format, duration_micros))
}
