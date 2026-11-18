use rust_decimal::Decimal;

use super::BudgetCascade;
use crate::spend_reset_unit_of_work::queue::{
    BudgetIdFilter, KeyLinkedResetFilter, KeyLinkedSpendResetWrites, LinkedResetFilter,
    LinkedSpendResetWrites, SpendRange,
};

// The two writes each linked table accepts, so the queue can be recorded in tests without a database.
pub trait LinkedResets {
    fn zero(&self, filter: LinkedResetFilter);
    fn decrement(&self, filter: LinkedResetFilter, amount: Decimal);
}

impl LinkedResets for LinkedSpendResetWrites {
    fn zero(&self, filter: LinkedResetFilter) {
        self.queue_spend_zero(filter);
    }

    fn decrement(&self, filter: LinkedResetFilter, amount: Decimal) {
        self.queue_spend_decrement(filter, amount);
    }
}

impl LinkedResets for KeyLinkedSpendResetWrites {
    fn zero(&self, filter: LinkedResetFilter) {
        self.queue_spend_zero(KeyLinkedResetFilter {
            budget_id: filter.budget_id,
            spend: filter.spend,
        });
    }

    fn decrement(&self, filter: LinkedResetFilter, amount: Decimal) {
        self.queue_spend_decrement(
            KeyLinkedResetFilter {
                budget_id: filter.budget_id,
                spend: filter.spend,
            },
            amount,
        );
    }
}

// Zero the under-cap rows BEFORE decrementing the over-cap ones: the statements run in order inside one transaction, so the reverse order lets the zero re-match a row the decrement just moved into the (0, cap] range and erase its carried spend.
fn queue_rollover_tier<Sink>(writes: &Sink, budget_id: &str, cap: Decimal)
where
    Sink: LinkedResets,
{
    writes.zero(LinkedResetFilter {
        budget_id: BudgetIdFilter::Equals(budget_id.to_owned()),
        spend: SpendRange {
            greater_than: Some(Decimal::ZERO),
            less_than_or_equal: Some(cap),
        },
    });
    writes.decrement(
        LinkedResetFilter {
            budget_id: BudgetIdFilter::Equals(budget_id.to_owned()),
            spend: SpendRange {
                greater_than: Some(cap),
                less_than_or_equal: None,
            },
        },
        cap,
    );
}

pub fn queue_budget_linked_resets<Sink>(writes: &Sink, cascade: &BudgetCascade, extra: SpendRange)
where
    Sink: LinkedResets,
{
    for (budget_id, cap) in &cascade.rollover_caps {
        queue_rollover_tier(writes, budget_id, *cap);
    }
    let plain_ids: Vec<String> = cascade
        .budget_ids
        .iter()
        .filter(|budget_id| !cascade.rollover_caps.contains_key(*budget_id))
        .cloned()
        .collect();
    if plain_ids.is_empty() {
        return;
    }
    writes.zero(LinkedResetFilter {
        budget_id: BudgetIdFilter::In(plain_ids),
        spend: extra,
    });
}

mod tests;
