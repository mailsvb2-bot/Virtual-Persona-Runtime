use vpr_integration::{CancellationProbe, ProviderError};
use vpr_policy::{AuthorityScope, DataClass};

use crate::cancellation::ProviderCancellation;
use crate::error::{ProviderExecutionError, RuntimeDenyReason};

/// Internal linearized capability for one immediate provider operation.
///
/// It is never exposed to callers; public provider execution methods acquire and consume it
/// inside one runtime call, eliminating delayed or repeated permit use.
#[derive(Debug)]
pub(crate) struct ProviderExecutionPermit {
    pub(crate) cancellation: ProviderCancellation,
}

impl ProviderExecutionPermit {
    /// The provider can ignore cancellation and return a late success. Never
    /// promote that result into a runtime output or a new stream capability.
    /// This postflight is shared by owner and visitor turns; it is NOT a
    /// substitute for cancellation-aware transport dispatch or remote STOP.
    pub(crate) fn finish<T>(
        &self,
        result: Result<T, ProviderError>,
    ) -> Result<T, ProviderExecutionError> {
        if self.cancellation.is_cancelled() {
            return Err(ProviderExecutionError::Denied(RuntimeDenyReason::TurnCancelled));
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
