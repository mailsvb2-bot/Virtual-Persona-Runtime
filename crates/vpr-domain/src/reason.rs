#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rt0ReasonCode {
    AuthRevoked,
    AuthExpired,
    AuthScopeDenied,
    EgressDenied,
    EgressLocalOnly,
    ConsentRequired,
    ProviderUnavailable,
    ProviderRateLimited,
    ProviderTimeout,
    PreparationFailed,
    TurnCancelled,
    InvalidStateTransition,
    OwnerAttributionUnverified,
    BudgetExhausted,
    InternalError,
}

impl Rt0ReasonCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AuthRevoked => "AUTH_REVOKED",
            Self::AuthExpired => "AUTH_EXPIRED",
            Self::AuthScopeDenied => "AUTH_SCOPE_DENIED",
            Self::EgressDenied => "EGRESS_DENIED",
            Self::EgressLocalOnly => "EGRESS_LOCAL_ONLY",
            Self::ConsentRequired => "CONSENT_REQUIRED",
            Self::ProviderUnavailable => "PROVIDER_UNAVAILABLE",
            Self::ProviderRateLimited => "PROVIDER_RATE_LIMITED",
            Self::ProviderTimeout => "PROVIDER_TIMEOUT",
            Self::PreparationFailed => "PREPARATION_FAILED",
            Self::TurnCancelled => "TURN_CANCELLED",
            Self::InvalidStateTransition => "INVALID_STATE_TRANSITION",
            Self::OwnerAttributionUnverified => "OWNER_ATTRIBUTION_UNVERIFIED",
            Self::BudgetExhausted => "BUDGET_EXHAUSTED",
            Self::InternalError => "INTERNAL_ERROR",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reason_codes_match_rt0_release_contract() {
        assert_eq!(Rt0ReasonCode::AuthRevoked.as_str(), "AUTH_REVOKED");
        assert_eq!(Rt0ReasonCode::TurnCancelled.as_str(), "TURN_CANCELLED");
        assert_eq!(
            Rt0ReasonCode::OwnerAttributionUnverified.as_str(),
            "OWNER_ATTRIBUTION_UNVERIFIED"
        );
    }
}
