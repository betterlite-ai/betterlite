use rust_decimal::Decimal;

use super::BudgetCascade;
use super::rollover_order::{LinkedResets, queue_budget_linked_resets};
use crate::budget_reset_policy::filters::SPENT_ROWS;
use crate::spend_reset_unit_of_work::queue::{BudgetIdFilter, LinkedResetFilter, SpendRange};

// _enduser_invalidation_where: rows created implicitly persist no budget link and ride the configured default tier, whose NULL rows join the walk only when that tier is expiring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndUserBudgetMatch<'ids> {
    pub linked_budget_ids: &'ids [String],
    pub include_unlinked: bool,
}

#[must_use]
pub fn enduser_invalidation_match<'ids>(
    budget_ids: &'ids [String],
    default_enduser_budget_id: Option<&str>,
) -> EndUserBudgetMatch<'ids> {
    let includes_default = default_enduser_budget_id
        .is_some_and(|default_id| budget_ids.iter().any(|budget_id| budget_id == default_id));
    EndUserBudgetMatch {
        linked_budget_ids: budget_ids,
        include_unlinked: includes_default,
    }
}

fn queue_unlinked_endusers<Sink>(writes: &Sink, cap: Option<Decimal>)
where
    Sink: LinkedResets,
{
    let Some(cap_value) = cap else {
        writes.zero(LinkedResetFilter {
            budget_id: BudgetIdFilter::IsNull,
            spend: SPENT_ROWS,
        });
        return;
    };
    writes.zero(LinkedResetFilter {
        budget_id: BudgetIdFilter::IsNull,
        spend: SpendRange {
            greater_than: Some(Decimal::ZERO),
            less_than_or_equal: Some(cap_value),
        },
    });
    writes.decrement(
        LinkedResetFilter {
            budget_id: BudgetIdFilter::IsNull,
            spend: SpendRange {
                greater_than: Some(cap_value),
                less_than_or_equal: None,
            },
        },
        cap_value,
    );
}

// End users reset on the budget link like every other gated table, plus the NULL-budget_id branch the default tier owns; matching on the link keeps the bind count proportional to the expiring tiers instead of the customer population.
pub fn queue_enduser_resets<Sink>(
    writes: &Sink,
    cascade: &BudgetCascade,
    default_enduser_budget_id: Option<&str>,
) where
    Sink: LinkedResets,
{
    queue_budget_linked_resets(writes, cascade, SPENT_ROWS);
    if !enduser_invalidation_match(&cascade.budget_ids, default_enduser_budget_id).include_unlinked
    {
        return;
    }
    let Some(default_budget_id) = default_enduser_budget_id else {
        return;
    };
    queue_unlinked_endusers(
        writes,
        cascade.rollover_caps.get(default_budget_id).copied(),
    );
}

mod tests;
