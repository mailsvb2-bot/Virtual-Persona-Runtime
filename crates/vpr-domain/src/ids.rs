use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdError;

impl Display for IdError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("canonical identifiers must not be empty")
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
                if value.trim().is_empty() {
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
canonical_id!(SessionId);
canonical_id!(TurnId);
canonical_id!(CorrelationId);
