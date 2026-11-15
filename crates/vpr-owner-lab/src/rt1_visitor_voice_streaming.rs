//! Canonical RT1 visitor streaming voice through the verified worker owner.
use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::{Rt1VisitorConversation, unsupported_visitor_voice_egress};
use crate::state::{LabVoiceResult, LabError};

impl Rt1VisitorConversation {
    /// Run the canonical pull-stream STT -> LLM -> TTS path for a verified
    /// worker visitor. The existing `OwnerLabEngine` streams each synthesized
    /// phrase to `OperationWorkerAvatar`, which hands PCM to the same durable
    /// worker operation; this method creates no browser-side or second runtime.
    ///
    /// The callback is deliberately internal and accepts only segments without
    /// client commands: audio must leave through the worker's authenticated
    /// RTP senders, never through an untracked browser command.
    ///
    /// # Errors
    /// Denies diagnostic/unverified sessions, stale publication, cancellation,
    /// invalid microphone audio, or any provider/client-control failure.
    pub fn voice_turn_streaming(
        &self,
        pcm_s16le_mono_16khz: Vec<u8>,
    ) -> Result<LabVoiceResult, String> {
        if !self.supports_private_voice() {
            return unsupported_visitor_voice_egress();
        }
        // Stage the exact PCM under the canonical publication transaction.
        // This is a bounded local handoff, not permission for provider egress.
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        self.stage_microphone_audio(&sender, pcm_s16le_mono_16khz)?;
        let pcm_s16le_mono_16khz = receiver
            .try_recv()
            .map_err(|_| "RT1_MICROPHONE_QUEUE_UNAVAILABLE".to_owned())?;
        // The same revocation-serialized permit protects every STT/LLM/TTS
        // provider operation and the worker PCM delivery that follows it.
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
                    if segment.client_command.is_some() {
                        Err(LabError::InvalidState)
                    } else {
                        Ok(())
                    }
                },
            )
            .map_err(|error| error.code().to_owned());
        *self.interrupt.turn.lock() = None;
        // Publication revoke wins over a late successful stream completion.
        self.manager.authorize(&self.id)?;
        if self.interrupt.cancelled.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        result
    }
}
