use std::collections::BTreeMap;

use rust_decimal::Decimal;

use crate::budget_counter::BudgetCounter;

// _reservation_groups: everything with room for the estimate is charged in one pipeline, else one counter at a time so the policy settles each before the next is touched
pub(crate) fn reservation_groups<'a>(
    counters: &[&'a BudgetCounter],
    current_spend_by_counter_key: &BTreeMap<String, Decimal>,
    reservation_cost: Decimal,
) -> Vec<Vec<&'a BudgetCounter>> {
    if counters.is_empty() {
        return Vec::new();
    }
    if everything_fits(counters, current_spend_by_counter_key, reservation_cost) {
        return vec![counters.to_vec()];
    }
    counters.iter().map(|counter| vec![*counter]).collect()
}

fn everything_fits(
    counters: &[&BudgetCounter],
    current_spend_by_counter_key: &BTreeMap<String, Decimal>,
    reservation_cost: Decimal,
) -> bool {
    for counter in counters {
        let Some(current_spend) = current_spend_by_counter_key.get(&counter.counter_key) else {
            return false;
        };
        let fits = current_spend
            .checked_add(reservation_cost)
            .is_some_and(|total| total <= counter.max_budget);
        if !fits {
            return false;
        }
    }
    true
}
