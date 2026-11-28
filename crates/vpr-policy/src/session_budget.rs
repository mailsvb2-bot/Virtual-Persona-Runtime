//! Integer-denominated budget reservations for the canonical session owner.
//! Persist the ledger atomically before provider egress. Never infer charges.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reservation {
    maximum: u64,
    confirmed: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionBudget {
    limit: u64,
    operations: BTreeMap<String, Reservation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetDenial {
    InvalidOperation,
    DuplicateOperation,
    Exhausted,
    UnknownReservation,
    ConflictingReceipt,
    AboveReservation,
    InvalidLedger,
}

impl SessionBudget {
    #[must_use]
    pub fn new(limit: u64) -> Self {
        Self {
            limit,
            operations: BTreeMap::new(),
        }
    }

    /// Unknown provider outcomes remain reserved at their full upper bound.
    ///
    /// # Errors
    /// Denies corrupt or overflowing persisted accounting.
    pub fn remaining(&self) -> Result<u64, BudgetDenial> {
        let used = self.operations.values().try_fold(0_u64, |sum, entry| {
            if entry.maximum == 0 || entry.confirmed.is_some_and(|v| v > entry.maximum) {
                return Err(BudgetDenial::InvalidLedger);
            }
            sum.checked_add(entry.confirmed.unwrap_or(entry.maximum))
                .ok_or(BudgetDenial::InvalidLedger)
        })?;
        self.limit
            .checked_sub(used)
            .ok_or(BudgetDenial::InvalidLedger)
    }

    /// Reserve once; a repeated key never authorizes another external effect.
    /// Caller must serialize and durably commit before dispatch.
    ///
    /// # Errors
    /// Denies invalid identifiers, duplicates and insufficient funds.
    pub fn reserve(&mut self, operation: &str, maximum: u64) -> Result<(), BudgetDenial> {
        if operation.trim().is_empty() || maximum == 0 {
            return Err(BudgetDenial::InvalidOperation);
        }
        if self.operations.contains_key(operation) {
            return Err(BudgetDenial::DuplicateOperation);
        }
        if maximum > self.remaining()? {
            return Err(BudgetDenial::Exhausted);
        }
        self.operations.insert(
            operation.to_owned(),
            Reservation {
                maximum,
                confirmed: None,
            },
        );
        Ok(())
    }

    /// Record an authenticated, final provider charge. Replaying the same
    /// receipt is harmless. An unknown outcome must not call this method.
    ///
    /// # Errors
    /// Denies missing reservations, conflicting or above-bound receipts.
    pub fn settle(&mut self, operation: &str, amount: u64) -> Result<(), BudgetDenial> {
        let entry = self
            .operations
            .get_mut(operation)
            .ok_or(BudgetDenial::UnknownReservation)?;
        if let Some(previous) = entry.confirmed {
            return if previous == amount {
                Ok(())
            } else {
                Err(BudgetDenial::ConflictingReceipt)
            };
        }
        if amount > entry.maximum {
            return Err(BudgetDenial::AboveReservation);
        }
        entry.confirmed = Some(amount);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_outcomes_retain_funds_and_retries_never_dispatch_twice() {
        let mut budget = SessionBudget::new(100);
        assert_eq!(budget.reserve("a", 70), Ok(()));
        assert_eq!(
            budget.reserve("a", 70),
            Err(BudgetDenial::DuplicateOperation)
        );
        assert_eq!(budget.reserve("b", 31), Err(BudgetDenial::Exhausted));
        let raw = serde_json::to_string(&budget).unwrap();
        let mut budget: SessionBudget = serde_json::from_str(&raw).unwrap();
        assert_eq!(budget.remaining(), Ok(30));
        assert_eq!(budget.settle("a", 71), Err(BudgetDenial::AboveReservation));
        assert_eq!(budget.settle("a", 20), Ok(()));
        assert_eq!(budget.settle("a", 20), Ok(()));
        assert_eq!(
            budget.settle("a", 21),
            Err(BudgetDenial::ConflictingReceipt)
        );
        assert_eq!(budget.reserve("b", 80), Ok(()));
        assert_eq!(budget.remaining(), Ok(0));
    }
}
