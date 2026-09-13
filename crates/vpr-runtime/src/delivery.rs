use vpr_domain::{Rt0ReasonCode, TurnId};
use vpr_integration::{
    RealtimeOutputPort, RealtimeTextOutputEvent, TransportError, TransportErrorKind,
};

use crate::{ActiveTurn, OutputSegmentId};

#[derive(Debug, PartialEq, Eq)]
pub struct OutputDeliveryHandle {
    turn_id: TurnId,
    segment_id: OutputSegmentId,
}

impl OutputDeliveryHandle {
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.segment_id.get()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum OutputDeliveryError {
    Runtime(Rt0ReasonCode),
    Transport(TransportError),
    DeliveryUncertain {
        error: TransportError,
        handle: OutputDeliveryHandle,
    },
}

impl OutputDeliveryError {
    #[must_use]
    pub const fn reason_code(&self) -> Rt0ReasonCode {
        match self {
            Self::Runtime(reason) => *reason,
            Self::Transport(error) | Self::DeliveryUncertain { error, .. } => match error.kind {
                TransportErrorKind::Cancelled => Rt0ReasonCode::TurnCancelled,
                TransportErrorKind::Unavailable | TransportErrorKind::DeliveryUncertain => {
                    Rt0ReasonCode::InternalError
                }
            },
        }
    }

    #[must_use]
    pub const fn uncertain_handle(&self) -> Option<&OutputDeliveryHandle> {
        match self {
            Self::DeliveryUncertain { handle, .. } => Some(handle),
            Self::Runtime(_) | Self::Transport(_) => None,
        }
    }
}

impl From<Rt0ReasonCode> for OutputDeliveryError {
    fn from(value: Rt0ReasonCode) -> Self {
        Self::Runtime(value)
    }
}

impl From<TransportError> for OutputDeliveryError {
    fn from(value: TransportError) -> Self {
        Self::Transport(value)
    }
}

impl ActiveTurn {
    /// Sends generated text through the canonical transport boundary and records `Sent` only after
    /// a positive transport acknowledgement.
    ///
    /// # Errors
    /// Returns a runtime lifecycle error or a typed transport error. Failed/uncertain transport
    /// calls never fabricate a `Sent` checkpoint.
    pub fn deliver_text(
        &self,
        port: &dyn RealtimeOutputPort,
        text: &str,
    ) -> Result<OutputDeliveryHandle, OutputDeliveryError> {
        let segment_id = self.begin_output_segment()?;
        let event = RealtimeTextOutputEvent {
            turn_id: self.snapshot.turn_id().clone(),
            correlation_id: self.snapshot.correlation_id().clone(),
            sequence: segment_id.get(),
            text: text.to_owned(),
        };
        let handle = OutputDeliveryHandle {
            turn_id: self.snapshot.turn_id().clone(),
            segment_id,
        };
        match port.send_text(&event, &self.cancellation) {
            Ok(()) => {
                self.state.lock().reconcile_output_sent(segment_id)?;
                Ok(handle)
            }
            Err(error) if error.kind == TransportErrorKind::DeliveryUncertain => {
                self.state
                    .lock()
                    .reconcile_output_delivery_uncertain(segment_id)?;
                Err(OutputDeliveryError::DeliveryUncertain { error, handle })
            }
            Err(error) => Err(OutputDeliveryError::Transport(error)),
        }
    }

    /// Applies a confirmed transport-send acknowledgement to a runtime-issued handle.
    ///
    /// This is primarily used to resolve a prior `DeliveryUncertain` result and is idempotent.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` for a handle from another turn or invalid checkpoint.
    pub fn acknowledge_output_sent(
        &self,
        handle: &OutputDeliveryHandle,
    ) -> Result<(), Rt0ReasonCode> {
        self.validate_delivery_handle(handle)?;
        self.state.lock().reconcile_output_sent(handle.segment_id)
    }

    /// Applies a participant playback acknowledgement for a handle issued by `deliver_text`.
    ///
    /// A late acknowledgement after interruption reconciles the cancelled checkpoint instead of
    /// reanimating the turn. The operation is idempotent for duplicate playback receipts.
    ///
    /// # Errors
    /// Returns `INVALID_STATE_TRANSITION` for a handle from another turn or invalid checkpoint.
    pub fn acknowledge_output_played(
        &self,
        handle: &OutputDeliveryHandle,
    ) -> Result<(), Rt0ReasonCode> {
        self.validate_delivery_handle(handle)?;
        self.state.lock().reconcile_output_played(handle.segment_id)
    }

    fn validate_delivery_handle(&self, handle: &OutputDeliveryHandle) -> Result<(), Rt0ReasonCode> {
        if &handle.turn_id == self.snapshot.turn_id() {
            Ok(())
        } else {
            Err(Rt0ReasonCode::InvalidStateTransition)
        }
    }
}
