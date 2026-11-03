use std::sync::{Arc, Mutex, PoisonError};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use super::SpendResetData;

#[derive(Debug)]
pub(super) struct Queue<T> {
    pub(super) entries: Arc<Mutex<Vec<T>>>,
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self {
            entries: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl<T> Clone for Queue<T> {
    fn clone(&self) -> Self {
        Self {
            entries: Arc::clone(&self.entries),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct SpendResetEntry {
    pub(super) table: &'static str,
    pub(super) column: &'static str,
    pub(super) identifier: String,
    pub(super) data: SpendResetData,
}

macro_rules! single_reset_writes {
    ($name:ident, $table:expr, $column:expr) => {
        #[derive(Debug, Default, Clone)]
        pub struct $name {
            pub(super) queue: Queue<SpendResetEntry>,
        }

        impl $name {
            pub fn queue_spend_reset(
                &self,
                identifier: &str,
                budget_reset_at: Option<DateTime<Utc>>,
                spend_decrement: Decimal,
            ) {
                let data = super::spend_reset_data(budget_reset_at, spend_decrement);
                let mut entries = self
                    .queue
                    .entries
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                entries.push(SpendResetEntry {
                    table: $table,
                    column: $column,
                    identifier: identifier.to_owned(),
                    data,
                });
            }
        }
    };
}

single_reset_writes!(KeySpendResetWrites, "LiteLLM_VerificationToken", "token");
single_reset_writes!(UserSpendResetWrites, "LiteLLM_UserTable", "user_id");
single_reset_writes!(TeamSpendResetWrites, "LiteLLM_TeamTable", "team_id");

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetIdFilter {
    Equals(String),
    In(Vec<String>),
    IsNull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpendRange {
    pub greater_than: Option<Decimal>,
    pub less_than_or_equal: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedResetFilter {
    pub budget_id: BudgetIdFilter,
    pub spend: SpendRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyLinkedResetFilter {
    pub budget_id: BudgetIdFilter,
    pub spend: SpendRange,
}

#[derive(Debug, Clone)]
pub(super) enum LinkedWrite {
    Zero(LinkedResetFilter),
    Decrement(LinkedResetFilter, Decimal),
    KeyZero(KeyLinkedResetFilter),
    KeyDecrement(KeyLinkedResetFilter, Decimal),
}

#[derive(Debug, Clone)]
pub(super) enum CascadeEntry {
    Linked {
        table: &'static str,
        write: LinkedWrite,
    },
    Window {
        budget_id: String,
        budget_reset_at: DateTime<Utc>,
    },
}

#[derive(Debug, Clone)]
pub struct LinkedSpendResetWrites {
    pub(super) table: &'static str,
    pub(super) queue: Queue<CascadeEntry>,
}

#[derive(Debug, Clone)]
pub struct KeyLinkedSpendResetWrites {
    pub(super) table: &'static str,
    pub(super) queue: Queue<CascadeEntry>,
}

macro_rules! linked_reset_writes {
    ($writes:ty, $filter:ty, $zero:ident, $decrement:ident) => {
        impl $writes {
            pub fn queue_spend_zero(&self, filter: $filter) {
                let entry = CascadeEntry::Linked {
                    table: self.table,
                    write: LinkedWrite::$zero(filter),
                };
                self.queue
                    .entries
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(entry);
            }
            pub fn queue_spend_decrement(&self, filter: $filter, amount: Decimal) {
                let entry = CascadeEntry::Linked {
                    table: self.table,
                    write: LinkedWrite::$decrement(filter, amount),
                };
                self.queue
                    .entries
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(entry);
            }
        }
    };
}
linked_reset_writes!(LinkedSpendResetWrites, LinkedResetFilter, Zero, Decrement);
linked_reset_writes!(
    KeyLinkedSpendResetWrites,
    KeyLinkedResetFilter,
    KeyZero,
    KeyDecrement
);

#[derive(Debug, Clone)]
pub struct BudgetWindowWrites {
    pub(super) queue: Queue<CascadeEntry>,
}

impl BudgetWindowWrites {
    pub fn queue_window_advance(&self, budget_id: &str, budget_reset_at: DateTime<Utc>) {
        let owned_budget_id = budget_id.to_owned();
        let mut entries = self
            .queue
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        entries.push(CascadeEntry::Window {
            budget_id: owned_budget_id,
            budget_reset_at,
        });
    }
}
