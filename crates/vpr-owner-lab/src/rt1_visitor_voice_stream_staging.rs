//! Internal canonical RT1 streaming bridge. Phrase events are staged only
//! into the publication-authorized bounded local queue; no browser delivery.
use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::{Rt1VisitorConversation, unsupported_visitor_voice_egress};
use crate::state::{LabVoiceResult, LabVoiceSegment};

impl Rt1VisitorConversation {
    /// Run the existing canonical STT -> LLM -> avatar streaming engine, but
    /// stage its phrase events ONLY in a bounded, local queue. This is an
    /// internal pre-delivery bridge, not a browser/provider egress callback.
    ///
    /// The consumer MUST separately prove a revocation-serialized final
    /// delivery/STOP fence before forwarding any queued command to a browser.
    /// A full/disconnected queue fails the canonical turn instead of spawning
    /// an unbounded producer or waiting behind a stalled consumer.
    ///
    /// # Errors
    /// Denies unsupported senders, revoked publication, malformed microphone
    /// input, queue backpressure, and canonical provider/runtime failures.
    #[allow(dead_code)] // No public delivery route until #319 is closed.
    pub(crate) fn voice_turn_streaming_staged(
        &self,
        pcm_s16le_mono_16khz: Vec<u8>,
        sender: &std::sync::mpsc::SyncSender<LabVoiceSegment>,
    ) -> Result<LabVoiceResult, String> {
        if self.remote.is_some()
            || self.interrupt.cancelled.load(Ordering::Acquire)
            || self.interrupt.backend_stop.lock().is_none()
        {
            return unsupported_visitor_voice_egress();
        }
        // Match the buffered RT1 voice path: admission and PCM handoff are
        // local-only and serialized with the owner's publication transaction.
        let (input_sender, input_receiver) = std::sync::mpsc::sync_channel(1);
        self.stage_microphone_audio(&input_sender, pcm_s16le_mono_16khz)?;
        let pcm_s16le_mono_16khz = input_receiver
            .try_recv()
            .map_err(|_| "RT1_MICROPHONE_QUEUE_UNAVAILABLE".to_owned())?;
        let _egress = self.manager.begin_provider_egress(&self.id)?;
        if self.interrupt.cancelled.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_REVOKED".into());
        }

        let callback = Arc::clone(&self.interrupt);
        let result = self
            .engine
            .lock()
            .voice_turn_streaming(
                pcm_s16le_mono_16khz,
                move |handle| {
                    *callback.turn.lock() = Some(handle.clone());
                    if callback.cancelled.load(Ordering::Acquire) {
                        let _ = handle.interrupt();
                    }
                },
                |segment| {
                    // Never call an arbitrary user callback or send externally
                    // from the canonical engine's streaming producer.
                    self.manager
                        .enqueue_media_fragment(&self.id, sender, segment)
                        .map_err(|_| crate::state::LabError::InvalidState)
                },
            )
            .map_err(|error| error.code().to_owned());
        *self.interrupt.turn.lock() = None;
        self.manager.authorize(&self.id)?;
        if self.interrupt.cancelled.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        result
    }
}
