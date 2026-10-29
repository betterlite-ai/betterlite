use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use super::SpendResetData;

#[derive(Debug, Clone)]
pub(super) struct SingleReset {
    pub(super) identifier: String,
    pub(super) data: SpendResetData,
}

macro_rules! single_reset_writes {
    ($name:ident) => {
        #[derive(Debug, Default)]
        pub struct $name {
            pub(super) queued: Vec<SingleReset>,
        }

        impl $name {
            pub fn queue_spend_reset(
                &mut self,
                identifier: &str,
                budget_reset_at: Option<DateTime<Utc>>,
                spend_decrement: Decimal,
            ) {
                self.queued.push(SingleReset {
                    identifier: identifier.to_owned(),
                    data: super::spend_reset_data(budget_reset_at, spend_decrement),
                });
            }
        }
    };
}

single_reset_writes!(KeySpendResetWrites);
single_reset_writes!(UserSpendResetWrites);
single_reset_writes!(TeamSpendResetWrites);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetIdFilter {
    Equals(String),
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
    pub spend: Option<SpendRange>,
}

#[derive(Debug, Clone)]
pub(super) enum LinkedWrite {
    Zero(LinkedResetFilter),
    Decrement(LinkedResetFilter, Decimal),
}

#[derive(Debug, Default)]
pub struct LinkedSpendResetWrites {
    pub(super) queued: Vec<LinkedWrite>,
}

impl LinkedSpendResetWrites {
    pub fn queue_spend_zero(&mut self, filter: LinkedResetFilter) {
        self.queued.push(LinkedWrite::Zero(filter));
    }

    pub fn queue_spend_decrement(&mut self, filter: LinkedResetFilter, amount: Decimal) {
        self.queued.push(LinkedWrite::Decrement(filter, amount));
    }
}

#[derive(Debug, Clone)]
pub(super) struct WindowAdvance {
    pub(super) budget_id: String,
    pub(super) budget_reset_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub struct BudgetWindowWrites {
    pub(super) queued: Vec<WindowAdvance>,
}

impl BudgetWindowWrites {
    pub fn queue_window_advance(&mut self, budget_id: &str, budget_reset_at: DateTime<Utc>) {
        self.queued.push(WindowAdvance {
            budget_id: budget_id.to_owned(),
            budget_reset_at,
        });
    }
}
