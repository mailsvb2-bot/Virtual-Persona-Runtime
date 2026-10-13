//! One private PCM16 stream; stop/cancel always fences the same Echo worker.
use super::{SessionWorker, SPEECH_TIMEOUT, INTERRUPT_TIMEOUT};
use crate::{cancelled, invalid_response};
use std::fmt::Write as _;
use std::sync::Arc;
use serde_json::json;
use vpr_integration::{CancellationProbe, ProviderError, RealtimeAvatarPcmStream};

/// One canonical utterance, sent as D-ID documented `pcm16` byte-stream.
/// A dropped/failed stream always sends an interrupt or kills its private worker.
pub(super) struct EchoPythonPcmStream {
    pub(super) worker: Arc<SessionWorker>,
    pub(super) stream_id: u64,
    pub(super) rate: u32,
    pub(super) channels: u16,
    pub(super) bytes: usize,
    pub(super) closed: bool,
}

impl RealtimeAvatarPcmStream for EchoPythonPcmStream {
    fn push_pcm(
        &mut self,
        pcm: &[u8],
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        let frame = usize::from(self.channels) * 2;
        let next = self.bytes.checked_add(pcm.len()).ok_or_else(invalid_response)?;
        if self.closed || pcm.is_empty() || pcm.len() % frame != 0
            || next > 8 * 1024 * 1024 - 44
            || next / frame > self.rate as usize * 60
            || cancellation.is_cancelled()
        {
            return Err(if cancellation.is_cancelled() { cancelled() } else { invalid_response() });
        }
        for part in pcm.chunks(8 * 1024) {
            let mut hex = String::with_capacity(part.len() * 2);
            for byte in part {
                write!(&mut hex, "{byte:02x}").map_err(|_| invalid_response())?;
            }
            self.worker.request(
                json!({"command":"pcm_chunk","stream_id":self.stream_id,"pcm_hex":hex}),
                SPEECH_TIMEOUT,
                Some(cancellation),
                false,
            )?;
        }
        self.bytes = next;
        Ok(())
    }

    fn finish(&mut self, cancellation: &dyn CancellationProbe) -> Result<(), ProviderError> {
        if self.closed || self.bytes == 0 || cancellation.is_cancelled() {
            return Err(if cancellation.is_cancelled() { cancelled() } else { invalid_response() });
        }
        // "End" is acknowledged only after LiveKit closes the byte stream.
        // It is NOT a D-ID playout or a browser hearing acknowledgement.
        self.worker.request(
            json!({"command":"pcm_end","stream_id":self.stream_id}),
            SPEECH_TIMEOUT,
            Some(cancellation),
            false,
        )?;
        self.closed = true;
        Ok(())
    }

    fn abort(&mut self) -> Result<(), ProviderError> {
        if self.closed {
            return Ok(());
        }
        let result = self.worker.request(
            json!({"command":"pcm_abort","stream_id":self.stream_id}),
            INTERRUPT_TIMEOUT,
            None,
            true,
        );
        self.closed = true;
        if result.is_err() {
            // A missing STOP receipt is unsafe; the private sender must die.
            let _ = self.worker.terminate();
        }
        result
    }
}

impl Drop for EchoPythonPcmStream {
    fn drop(&mut self) {
        if !self.closed {
            let _ = self.abort();
        }
    }
}

