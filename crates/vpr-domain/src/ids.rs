use std::error::Error;
use std::fmt::{Display, Formatter};

pub const MAX_CANONICAL_ID_CHARS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdError;

impl Display for IdError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("canonical identifiers must be non-empty and at most 256 characters")
    }
}

impl Error for IdError {}

macro_rules! canonical_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// Creates a non-empty canonical identifier.
            ///
            /// # Errors
            /// Returns `IdError` when the supplied identifier is blank.
            pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                if value.trim().is_empty() || value.chars().count() > MAX_CANONICAL_ID_CHARS {
                    return Err(IdError);
                }
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

canonical_id!(PersonaId);
canonical_id!(ClaimId);
canonical_id!(PreparationJobId);
canonical_id!(SessionId);
canonical_id!(TurnId);
canonical_id!(CorrelationId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AuthorizationEpoch(u64);

impl AuthorizationEpoch {
    #[must_use]
    pub const fn initial() -> Self {
        Self(1)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns the next authority revision.
    ///
    /// # Errors
    /// Returns `AuthorizationEpochExhausted` if the numeric epoch cannot advance.
    pub fn next(self) -> Result<Self, AuthorizationEpochExhausted> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(AuthorizationEpochExhausted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorizationEpochExhausted;

impl Display for AuthorizationEpochExhausted {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("authorization epoch exhausted")
    }
}

impl Error for AuthorizationEpochExhausted {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PolicyRevision(u64);

impl PolicyRevision {
    #[must_use]
    pub const fn initial() -> Self {
        Self(1)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns the next policy revision.
    ///
    /// # Errors
    /// Returns `PolicyRevisionExhausted` if the numeric revision cannot advance.
    pub fn next(self) -> Result<Self, PolicyRevisionExhausted> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(PolicyRevisionExhausted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicyRevisionExhausted;

impl Display for PolicyRevisionExhausted {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("policy revision exhausted")
    }
}

impl Error for PolicyRevisionExhausted {}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_ids_reject_blank_and_oversized_values() {
        assert!(PersonaId::new("   ").is_err());
        assert!(PersonaId::new("x".repeat(MAX_CANONICAL_ID_CHARS)).is_ok());
        assert!(PersonaId::new("x".repeat(MAX_CANONICAL_ID_CHARS + 1)).is_err());
    }

    #[test]
    fn canonical_id_limit_counts_unicode_scalars_not_utf8_bytes() {
        assert!(ClaimId::new("Ж".repeat(MAX_CANONICAL_ID_CHARS)).is_ok());
        assert!(ClaimId::new("Ж".repeat(MAX_CANONICAL_ID_CHARS + 1)).is_err());
    }
}
