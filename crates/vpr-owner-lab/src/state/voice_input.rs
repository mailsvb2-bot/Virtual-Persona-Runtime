use std::sync::Arc;

use vpr_integration::{PcmSampleFormat, SttStreamRequest};
use vpr_runtime::{ActiveTurn, TurnInterruptHandle};

use super::{
    LabError, OwnerLabEngine, voice::terminalize_failed_turn, voice_stt::VoiceSttInput,
};

const VOICE_SAMPLE_RATE_HZ: u32 = 16_000;
const VOICE_CHANNELS: u16 = 1;
const MAX_VOICE_BYTES: usize = 16_000 * 2 * 30;

pub struct LabVoiceInput {
    pub(super) turn: Arc<ActiveTurn>,
    pub(super) evidence_turn_sequence: u64,
    pub(super) stt_input: VoiceSttInput,
    received_bytes: usize,
}

impl LabVoiceInput {
    /// Pushes canonical S16LE/mono/16 kHz microphone bytes into the already-authorized STT turn.
    ///
    /// # Errors
    /// Fails closed on malformed framing, the 30-second RT0 bound, provider failure, revocation,
    /// expiry, or interruption.
    pub fn push_audio(&mut self, pcm: &[u8]) -> Result<(), LabError> {
        let next_bytes = self
            .received_bytes
            .checked_add(pcm.len())
            .ok_or_else(|| terminalize_failed_turn(&self.turn, LabError::InvalidInput))?;
        if next_bytes > MAX_VOICE_BYTES {
            return Err(terminalize_failed_turn(
                &self.turn,
                LabError::InvalidInput,
            ));
        }
        self.stt_input.push_audio(&self.turn, pcm)?;
        self.received_bytes = next_bytes;
        Ok(())
    }

    #[must_use]
    pub fn received_bytes(&self) -> usize {
        self.received_bytes
    }

    pub fn abort(self, error: LabError) -> LabError {
        terminalize_failed_turn(&self.turn, error)
    }
}

impl OwnerLabEngine {
    /// Opens the canonical voice turn and STT stream before the browser has finished speaking.
    ///
    /// The returned input owns the runtime-authorized STT operation. Browser PCM may therefore be
    /// forwarded incrementally while the microphone is active without exposing provider transport
    /// details or creating a second authorization path.
    ///
    /// # Errors
    /// Fails closed when voice providers/session state are unavailable or current policy denies
    /// the STT provider operation.
    pub fn begin_voice_input(
        &mut self,
        register_interrupt: impl FnOnce(TurnInterruptHandle),
    ) -> Result<LabVoiceInput, LabError> {
        if self.stt.is_none() || self.llm.is_none() || self.avatar.is_none() {
            return Err(LabError::InvalidState);
        }

        let turn = Arc::new(self.new_turn()?);
        let evidence_turn_sequence = self.turn_counter;
        register_interrupt(turn.interrupt_handle());
        let stt = self.stt.as_ref().ok_or(LabError::InvalidState)?;
        let stt_input = VoiceSttInput::open(
            &turn,
            stt.as_ref(),
            SttStreamRequest {
                sample_rate_hz: VOICE_SAMPLE_RATE_HZ,
                channels: VOICE_CHANNELS,
                sample_format: PcmSampleFormat::S16Le,
                locale_hint: Some("ru-RU".to_owned()),
            },
        )?;

        Ok(LabVoiceInput {
            turn,
            evidence_turn_sequence,
            stt_input,
            received_bytes: 0,
        })
    }
}
