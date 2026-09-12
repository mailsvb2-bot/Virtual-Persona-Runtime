use vpr_domain::Rt0ReasonCode;
use vpr_integration::{ProviderError, ProviderErrorKind};
use vpr_policy::AuthorizationValidityError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeDenyReason {
    AuthorizationRevoked,
    AuthorizationExpired,
    AuthorizationStale,
    AuthorityDenied,
    EgressDenied,
    EgressPolicyStale,
    ConsentRequired,
    LocalOnlyRequired,
    InvalidTurnState,
    TurnCancelled,
    InternalError,
}

impl RuntimeDenyReason {
    #[must_use]
    pub const fn reason_code(self) -> Rt0ReasonCode {
        match self {
            Self::AuthorizationRevoked | Self::AuthorizationStale => Rt0ReasonCode::AuthRevoked,
            Self::AuthorizationExpired => Rt0ReasonCode::AuthExpired,
            Self::AuthorityDenied => Rt0ReasonCode::AuthScopeDenied,
            Self::EgressDenied | Self::EgressPolicyStale => Rt0ReasonCode::EgressDenied,
            Self::ConsentRequired => Rt0ReasonCode::ConsentRequired,
            Self::LocalOnlyRequired => Rt0ReasonCode::EgressLocalOnly,
            Self::InvalidTurnState => Rt0ReasonCode::InvalidStateTransition,
            Self::TurnCancelled => Rt0ReasonCode::TurnCancelled,
            Self::InternalError => Rt0ReasonCode::InternalError,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderExecutionError {
    Denied(RuntimeDenyReason),
    Provider(ProviderError),
}

impl ProviderExecutionError {
    #[must_use]
    pub const fn reason_code(&self) -> Rt0ReasonCode {
        match self {
            Self::Denied(reason) => reason.reason_code(),
            Self::Provider(error) => provider_reason_code(error.kind),
        }
    }
}

impl From<RuntimeDenyReason> for ProviderExecutionError {
    fn from(value: RuntimeDenyReason) -> Self {
        Self::Denied(value)
    }
}

impl From<ProviderError> for ProviderExecutionError {
    fn from(value: ProviderError) -> Self {
        Self::Provider(value)
    }
}

impl From<AuthorizationValidityError> for RuntimeDenyReason {
    fn from(value: AuthorizationValidityError) -> Self {
        match value {
            AuthorizationValidityError::StaleEpoch => Self::AuthorizationStale,
            AuthorizationValidityError::Revoked => Self::AuthorizationRevoked,
            AuthorizationValidityError::Expired => Self::AuthorizationExpired,
        }
    }
}

#[must_use]
pub const fn provider_reason_code(kind: ProviderErrorKind) -> Rt0ReasonCode {
    match kind {
        ProviderErrorKind::Unavailable => Rt0ReasonCode::ProviderUnavailable,
        ProviderErrorKind::RateLimited => Rt0ReasonCode::ProviderRateLimited,
        ProviderErrorKind::Timeout => Rt0ReasonCode::ProviderTimeout,
        ProviderErrorKind::Cancelled => Rt0ReasonCode::TurnCancelled,
        ProviderErrorKind::PolicyDenied => Rt0ReasonCode::EgressDenied,
        ProviderErrorKind::InvalidResponse => Rt0ReasonCode::InternalError,
    }
}
