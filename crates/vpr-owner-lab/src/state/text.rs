use std::time::Instant;

use serde::Serialize;
use std::sync::Mutex;

use vpr_integration::{
    CancellationProbe, LlmRequest, RealtimeOutputPort, RealtimeTextOutputEvent,
    TimedGeneratedTextBuffer, TransportError, TransportErrorKind,
};
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
    pub evidence_output_sequence: u64,
    pub first_meaningful_response_millis: u64,
    pub total_millis: u64,
    pub llm_usage: LabProviderUsage,
}

#[derive(Default)]
struct OwnerLabTextOutputPort {
    delivered: Mutex<Option<String>>,
}

impl OwnerLabTextOutputPort {
    fn take_delivered(&self) -> Result<String, LabError> {
        self.delivered
            .lock()
            .map_err(|_| LabError::Internal)?
            .take()
            .ok_or(LabError::Internal)
    }
}

impl RealtimeOutputPort for OwnerLabTextOutputPort {
    fn send_text(
        &self,
        event: &RealtimeTextOutputEvent,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), TransportError> {
        if cancellation.is_cancelled() {
            return Err(TransportError {
                kind: TransportErrorKind::Cancelled,
                retryable: false,
            });
        }
        let Ok(mut delivered) = self.delivered.lock() else {
            return Err(TransportError {
                kind: TransportErrorKind::Unavailable,
                retryable: false,
            });
        };
        if delivered.is_some() {
            return Err(TransportError {
                kind: TransportErrorKind::DeliveryUncertain,
                retryable: false,
            });
        }
        *delivered = Some(event.text.clone());
        Ok(())
    }
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
        if input.is_empty() || input.chars().count() > MAX_TEXT_CHARS {
            return Err(LabError::InvalidInput);
        }
        if self.llm.is_none() {
            return Err(LabError::InvalidState);
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
        let output = OwnerLabTextOutputPort::default();
        let delivery = turn
            .deliver_text(&output, &reply)
            .map_err(|error| {
                terminalize_failed_turn(&turn, LabError::Runtime(error.reason_code()))
            })?;
        let evidence_output_sequence = delivery.sequence();
        let reply = output.take_delivered()?;
        turn.complete().map_err(LabError::Runtime)?;
        Ok(LabTextResult {
            reply,
            locale: "ru-RU".to_owned(),
            evidence_turn_sequence,
            evidence_output_sequence,
            first_meaningful_response_millis,
            total_millis: elapsed_millis(total_started),
            llm_usage: map_usage(&llm_usage),
        })
    }
}
