use std::time::Instant;

use vpr_integration::{RealtimeAvatarPort, TtsPort};

use crate::{
    ActiveTurn, OutputDeliveryHandle, RealtimeAvatarHandle, RealtimeAvatarOutputError,
};

/// Monotonic elapsed milliseconds relative to the start of first phrase TTS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RealtimeAudioStageTimings {
    pub first_pcm_millis: u64,
    pub first_echo_send_millis: u64,
}

pub(crate) fn elapsed_stage(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

impl ActiveTurn {
    /// Backward-compatible canonical delivery, retaining exactly one measured implementation.
    ///
    /// # Errors
    /// Preserves the measured route's provider and authorization failures.
    pub fn synthesize_and_deliver_realtime_avatar_audio(
        &self,
        tts: &dyn TtsPort,
        port: &dyn RealtimeAvatarPort,
        handle: &RealtimeAvatarHandle,
        text: &str,
    ) -> Result<OutputDeliveryHandle, RealtimeAvatarOutputError> {
        self.synthesize_and_deliver_realtime_avatar_audio_with_timings(tts, port, handle, text)
            .map(|(delivery, _)| delivery)
    }
}
