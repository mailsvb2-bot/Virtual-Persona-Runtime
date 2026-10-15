use vpr_domain::Rt0ReasonCode;
use crate::ProviderExecutionError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealtimeAvatarOutputError {
    Runtime(Rt0ReasonCode),
    Provider(ProviderExecutionError),
}

impl RealtimeAvatarOutputError {
    #[must_use]
    pub const fn reason_code(&self) -> Rt0ReasonCode {
        match self {
            Self::Runtime(reason) => *reason,
            Self::Provider(error) => error.reason_code(),
        }
    }
}

