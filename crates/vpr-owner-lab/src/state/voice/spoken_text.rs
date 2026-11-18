//! Typed visitor questions reuse the *same* canonical LLM streaming,
//! TTS, avatar PCM and evidence path as microphone turns. No second runtime.
use super::{
    Arc, Instant, LabError, LabVoiceSegment, LlmRequest, OwnerLabEngine, TurnInterruptHandle,
    elapsed_millis, map_usage, terminalize_failed_turn,
};

impl OwnerLabEngine {
    /// Run a typed visitor question through the SAME streaming LLM -> TTS ->
    /// avatar PCM pipeline used by microphone turns, without requiring a
    /// fabricated STT input. A single canonical turn owns output evidence,
    /// cancellation and the existing server-side media sender.
    ///
    /// The caller must hold the canonical RT1 publication egress permit and
    /// reject any browser-side client command; this method never creates a
    /// second speech or avatar provider.
    ///
    /// # Errors
    /// Denies invalid text, unavailable providers/transport, cancelled
    /// sessions, LLM/TTS errors or uncertain media delivery.
    pub fn text_voice_turn_streaming(
        &mut self,
        input: &str,
        register_interrupt: impl FnOnce(TurnInterruptHandle),
        mut emit_segment: impl FnMut(LabVoiceSegment) -> Result<(), LabError>,
    ) -> Result<super::super::text::LabTextResult, LabError> {
        let input = input.trim();
        if input.is_empty() || input.chars().count() > 8_000 {
            return Err(LabError::InvalidInput);
        }
        if self.llm.is_none() || self.tts.is_none() || self.avatar.is_none() {
            return Err(LabError::InvalidState);
        }
        let total_started = Instant::now();
        let turn = Arc::new(self.new_turn()?);
        let evidence_turn_sequence = self.turn_counter;
        register_interrupt(turn.interrupt_handle());
        let user_input = self.memory_user_input(input)?;
        let _memory_egress = self.memory_provider_guard()?;
        let request = LlmRequest {
            locale: "ru-RU".to_owned(),
            instructions: Some(self.memory_instructions(input)?),
            user_input: user_input,
        };
        let llm = self.llm.as_ref().ok_or(LabError::InvalidState)?;
        let handle = self.avatar.as_ref().ok_or(LabError::InvalidState)?;
        let generation = self.streaming_generation(
            &turn,
            llm.as_ref(),
            handle,
            &request,
            evidence_turn_sequence,
            &mut emit_segment,
        )?;
        let evidence_output_sequence = generation
            .output_sequences
            .first()
            .copied()
            .ok_or_else(|| terminalize_failed_turn(&turn, LabError::InvalidInput))?;
        turn.complete().map_err(LabError::Runtime)?;
        self.memory_after_success(input, evidence_turn_sequence);
        Ok(super::super::text::LabTextResult {
            reply: generation.reply,
            locale: "ru-RU".to_owned(),
            evidence_turn_sequence,
            evidence_output_sequence,
            first_meaningful_response_millis: generation.first_meaningful_millis,
            total_millis: elapsed_millis(total_started),
            llm_usage: map_usage(&generation.usage),
        })
    }
}
