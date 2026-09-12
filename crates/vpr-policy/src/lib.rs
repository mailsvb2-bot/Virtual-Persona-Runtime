use std::collections::BTreeSet;
use vpr_domain::{AuthorizationEpoch, AuthorizationEpochExhausted};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AuthorityScope(String);

impl AuthorityScope {
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        (!value.trim().is_empty()).then_some(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthorityLayer {
    allowed: BTreeSet<AuthorityScope>,
    denied: BTreeSet<AuthorityScope>,
}

impl AuthorityLayer {
    #[must_use]
    pub fn new(
        allowed: impl IntoIterator<Item = AuthorityScope>,
        denied: impl IntoIterator<Item = AuthorityScope>,
    ) -> Self {
        Self {
            allowed: allowed.into_iter().collect(),
            denied: denied.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectiveAuthority {
    allowed: BTreeSet<AuthorityScope>,
}

impl EffectiveAuthority {
    #[must_use]
    pub fn compose(layers: &[AuthorityLayer]) -> Self {
        let Some(first) = layers.first() else {
            return Self::default();
        };

        let mut allowed = first.allowed.clone();
        let mut denied = first.denied.clone();
        for layer in &layers[1..] {
            allowed = allowed.intersection(&layer.allowed).cloned().collect();
            denied.extend(layer.denied.iter().cloned());
        }
        allowed.retain(|scope| !denied.contains(scope));
        Self { allowed }
    }

    #[must_use]
    pub fn allows(&self, scope: &AuthorityScope) -> bool {
        self.allowed.contains(scope)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataClass {
    Public,
    Internal,
    Personal,
    Biometric,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentState {
    Granted,
    Missing,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EgressRequest {
    pub data_class: DataClass,
    pub provider_policy_allows: bool,
    pub consent: ConsentState,
    pub local_only_required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EgressReason {
    Allowed,
    LocalOnlyRequired,
    ProviderPolicyBlocked,
    ConsentRequired,
    ConsentRevoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EgressDecision {
    Allow(EgressReason),
    LocalOnly(EgressReason),
    Deny(EgressReason),
}

#[must_use]
pub const fn decide_egress(request: EgressRequest) -> EgressDecision {
    if request.local_only_required {
        return EgressDecision::LocalOnly(EgressReason::LocalOnlyRequired);
    }
    if !request.provider_policy_allows {
        return EgressDecision::Deny(EgressReason::ProviderPolicyBlocked);
    }
    if matches!(request.data_class, DataClass::Biometric) {
        return match request.consent {
            ConsentState::Granted => EgressDecision::Allow(EgressReason::Allowed),
            ConsentState::Missing => EgressDecision::Deny(EgressReason::ConsentRequired),
            ConsentState::Revoked => EgressDecision::Deny(EgressReason::ConsentRevoked),
        };
    }
    if matches!(request.consent, ConsentState::Revoked) {
        return EgressDecision::Deny(EgressReason::ConsentRevoked);
    }
    EgressDecision::Allow(EgressReason::Allowed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorizationSnapshot {
    epoch: AuthorizationEpoch,
}

impl AuthorizationSnapshot {
    #[must_use]
    pub const fn epoch(&self) -> AuthorizationEpoch {
        self.epoch
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct AuthorizationState {
    epoch: AuthorizationEpoch,
    revoked: bool,
    expires_at_millis: Option<u64>,
}

impl AuthorizationState {
    #[must_use]
    pub const fn new(expires_at_millis: Option<u64>) -> Self {
        Self {
            epoch: AuthorizationEpoch::initial(),
            revoked: false,
            expires_at_millis,
        }
    }

    #[must_use]
    pub const fn snapshot(&self) -> AuthorizationSnapshot {
        AuthorizationSnapshot { epoch: self.epoch }
    }

    #[must_use]
    pub const fn epoch(&self) -> AuthorizationEpoch {
        self.epoch
    }

    #[must_use]
    pub const fn expires_at_millis(&self) -> Option<u64> {
        self.expires_at_millis
    }

    /// Revokes current authority and invalidates every earlier snapshot.
    ///
    /// # Errors
    /// Returns `AuthorizationEpochExhausted` if the revision cannot advance.
    pub fn revoke(&mut self) -> Result<(), AuthorizationEpochExhausted> {
        if !self.revoked {
            self.epoch = self.epoch.next()?;
            self.revoked = true;
        }
        Ok(())
    }

    /// Replaces authority with a new active epoch.
    ///
    /// # Errors
    /// Returns `AuthorizationEpochExhausted` if the revision cannot advance.
    pub fn replace(
        &mut self,
        expires_at_millis: Option<u64>,
    ) -> Result<(), AuthorizationEpochExhausted> {
        self.epoch = self.epoch.next()?;
        self.revoked = false;
        self.expires_at_millis = expires_at_millis;
        Ok(())
    }

    /// Validates that cached authority still belongs to the current active epoch.
    ///
    /// # Errors
    /// Fails closed for stale, revoked, or expired authority.
    pub const fn validate(
        &self,
        snapshot: AuthorizationSnapshot,
        now_millis: u64,
    ) -> Result<(), AuthorizationValidityError> {
        if snapshot.epoch().get() != self.epoch.get() {
            return Err(AuthorizationValidityError::StaleEpoch);
        }
        if self.revoked {
            return Err(AuthorizationValidityError::Revoked);
        }
        if let Some(expires_at_millis) = self.expires_at_millis
            && now_millis >= expires_at_millis
        {
            return Err(AuthorizationValidityError::Expired);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationValidityError {
    StaleEpoch,
    Revoked,
    Expired,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(value: &str) -> AuthorityScope {
        AuthorityScope::new(value).unwrap()
    }

    #[test]
    fn lower_policy_cannot_widen_authority() {
        let provider = scope("provider.egress");
        let voice = scope("owner.voice.use");
        let platform = AuthorityLayer::new([provider.clone()], []);
        let session = AuthorityLayer::new([provider.clone(), voice.clone()], []);
        let effective = EffectiveAuthority::compose(&[platform, session]);
        assert!(effective.allows(&provider));
        assert!(!effective.allows(&voice));
    }

    #[test]
    fn explicit_deny_wins() {
        let provider = scope("provider.egress");
        let platform = AuthorityLayer::new([provider.clone()], []);
        let session = AuthorityLayer::new([provider.clone()], [provider.clone()]);
        assert!(!EffectiveAuthority::compose(&[platform, session]).allows(&provider));
    }

    #[test]
    fn revoked_biometric_consent_fails_closed() {
        assert_eq!(
            decide_egress(EgressRequest {
                data_class: DataClass::Biometric,
                provider_policy_allows: true,
                consent: ConsentState::Revoked,
                local_only_required: false,
            }),
            EgressDecision::Deny(EgressReason::ConsentRevoked)
        );
    }

    #[test]
    fn local_only_never_becomes_external_allow() {
        assert_eq!(
            decide_egress(EgressRequest {
                data_class: DataClass::Public,
                provider_policy_allows: true,
                consent: ConsentState::Granted,
                local_only_required: true,
            }),
            EgressDecision::LocalOnly(EgressReason::LocalOnlyRequired)
        );
    }

    #[test]
    fn revocation_invalidates_cached_authorization_snapshot() {
        let mut state = AuthorizationState::new(None);
        let cached = state.snapshot();
        state.revoke().unwrap();
        assert_eq!(
            state.validate(cached, 0),
            Err(AuthorizationValidityError::StaleEpoch)
        );
    }

    #[test]
    fn current_authorization_expires_fail_closed() {
        let state = AuthorizationState::new(Some(100));
        assert_eq!(
            state.validate(state.snapshot(), 100),
            Err(AuthorizationValidityError::Expired)
        );
    }
}
