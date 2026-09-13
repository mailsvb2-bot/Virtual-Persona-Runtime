use vpr_domain::{CorrelationId, TurnId};

use crate::CancellationProbe;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealtimeTextOutputEvent {
    pub turn_id: TurnId,
    pub correlation_id: CorrelationId,
    pub sequence: u64,
    pub text: String,
}

pub trait RealtimeOutputPort: Send + Sync {
    /// Sends one canonical output event to the participant transport.
    ///
    /// Returning `Ok(())` is a positive transport-send acknowledgement. It does not imply that
    /// the participant has played/rendered the event; playback is a separate runtime receipt.
    ///
    /// # Errors
    /// Returns a typed transport failure. `DeliveryUncertain` means the caller must not blindly
    /// retry because the event may already have crossed the transport boundary.
    fn send_text(
        &self,
        event: &RealtimeTextOutputEvent,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), TransportError>;
}
