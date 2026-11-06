use vpr_integration::{CancellationProbe, ProviderError, ProviderErrorKind};
use vpr_policy::{AuthorityScope, DataClass};

use crate::cancellation::ProviderCancellation;
use crate::error::{ProviderExecutionError, RuntimeDenyReason};
use crate::execution_gate::ProviderEgressGuard;

/// Internal linearized capability for one immediate provider operation.
///
/// It is never exposed to callers; public provider execution methods acquire and consume it
/// inside one runtime call, eliminating delayed or repeated permit use.
#[derive(Debug)]
pub(crate) struct ProviderExecutionPermit {
    pub(crate) cancellation: ProviderCancellation,
    // Owns the lifetime of one canonical provider dispatch, including streams.
    // Drop means local I/O returned; it is not a remote audio/video STOP receipt.
    #[allow(dead_code)] // Held for its Drop side effect, never read as a value.
    pub(crate) active_egress: Option<ProviderEgressGuard>,
}

impl ProviderExecutionPermit {
    /// Check the same canonical cancellation before and after provider I/O.
    /// Even a misbehaving provider's late success is never authorized output.
    pub(crate) fn dispatch<T>(
        &self,
        operation: impl FnOnce(&dyn CancellationProbe) -> Result<T, ProviderError>,
    ) -> Result<T, ProviderExecutionError> {
        if self.cancellation.is_cancelled() {
            return Err(ProviderExecutionError::Denied(
                RuntimeDenyReason::TurnCancelled,
            ));
        }
        let result = operation(&self.cancellation);
        if self.cancellation.is_cancelled() {
            return match result {
                Err(error) if error.kind == ProviderErrorKind::Cancelled => {
                    Err(ProviderExecutionError::Provider(error))
                }
                _ => Err(ProviderExecutionError::Denied(
                    RuntimeDenyReason::TurnCancelled,
                )),
            };
        }
        result.map_err(ProviderExecutionError::from)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderOperation {
    Llm,
    Stt,
    Tts,
    Avatar,
}

impl ProviderOperation {
    /// RT0 fail-closed rule: until payload provenance can prove a lower class,
    /// every unstructured external-provider payload is treated as biometric.
    #[must_use]
    pub(crate) const fn data_class(self) -> DataClass {
        match self {
            Self::Llm | Self::Stt | Self::Tts | Self::Avatar => DataClass::Biometric,
        }
    }

    #[must_use]
    pub(crate) fn required_scope(self) -> AuthorityScope {
        let scope = match self {
            Self::Llm | Self::Stt | Self::Tts | Self::Avatar => "provider.egress",
        };
        AuthorityScope::new(scope).expect("canonical provider scope is non-empty")
    }
}
