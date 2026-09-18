use std::time::Instant;

use serde::Serialize;
use vpr_integration::{LlmRequest, TimedGeneratedTextBuffer};
use vpr_runtime::TurnInterruptHandle;

use super::voice::{
    LabProviderUsage, elapsed_millis, map_usage, terminalize_failed_turn,
    terminalize_provider_error,
};
use super::{LabError, OwnerLabEngine};

const MAX_TEXT_CHARS: usize = 8_000;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LabTextResult {
    pub reply: String,
    pub locale: String,
    pub evidence_turn_sequence: u64,
    pub first_meaningful_response_millis: u64,
    pub total_millis: u64,
    pub llm_usage: LabProviderUsage,
}

impl OwnerLabEngine {
    /// Runs one user text message through the canonical LLM turn without conflating it with the
    /// separate direct-avatar speak operation.
    ///
    /// The callback receives the same narrow interrupt-only capability used by voice turns before
    /// provider egress starts. The result contains no input text and only the user-visible reply
    /// plus sanitized timing/usage evidence.
    ///
    /// # Errors
    /// Fails closed for empty/oversized input, missing LLM readiness, inactive session/authority,
    /// cancellation, empty provider output, or a provider failure.
    pub fn text_turn(
        &mut self,
        input: String,
        register_interrupt: impl FnOnce(TurnInterruptHandle),
    ) -> Result<LabTextResult, LabError> {
        let input = input.trim();
        if input.is_empty() || input.chars().count() > MAX_TEXT_CHARS || self.llm.is_none() {
            return Err(LabError::InvalidInput);
        }

        let total_started = Instant::now();
        let mut generated = TimedGeneratedTextBuffer::start();
        let turn = self.new_turn()?;
        let evidence_turn_sequence = self.turn_counter;
        register_interrupt(turn.interrupt_handle());
        let context = self.conversation_context(input)?;
        let llm = self.llm.as_ref().ok_or(LabError::InvalidState)?;
        let llm_usage = turn
            .execute_llm(
                llm.as_ref(),
                &LlmRequest {
                    locale: "ru-RU".to_owned(),
                    context,
                },
                &mut generated,
            )
            .map_err(|error| terminalize_provider_error(&turn, error))?;
        let (reply, first_meaningful_response_millis) = generated.into_parts();
        let first_meaningful_response_millis =
            first_meaningful_response_millis.ok_or_else(|| {
                terminalize_failed_turn(&turn, LabError::InvalidInput)
            })?;
        if reply.trim().is_empty() {
            return Err(terminalize_failed_turn(&turn, LabError::InvalidInput));
        }

        turn.begin_output().map_err(LabError::Runtime)?;
        turn.complete().map_err(LabError::Runtime)?;
        Ok(LabTextResult {
            reply,
            locale: "ru-RU".to_owned(),
            evidence_turn_sequence,
            first_meaningful_response_millis,
            total_millis: elapsed_millis(total_started),
            llm_usage: map_usage(&llm_usage),
        })
    }
}
