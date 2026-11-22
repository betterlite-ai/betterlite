use rust_decimal::Decimal;

use crate::budget_counter::{BudgetCounter, CounterEntityType};

// one reserved counter of a budget reservation: LiteLLM's entry dict, typed
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReservationEntry {
    pub counter_key: Option<String>,
    pub entity_type: CounterEntityType,
    pub entity_id: String,
    pub reserved_cost: Option<Decimal>,
    pub applied_adjustment: Decimal,
}

// _counter_to_reservation_entry: a counter the reservation took at the pre-call estimate
#[must_use]
pub fn counter_to_reservation_entry(
    counter: &BudgetCounter,
    reserved_cost: Decimal,
) -> ReservationEntry {
    ReservationEntry {
        counter_key: Some(counter.counter_key.clone()),
        entity_type: counter.entity_type.clone(),
        entity_id: counter.entity_id.clone(),
        reserved_cost: Some(reserved_cost),
        applied_adjustment: Decimal::ZERO,
    }
}

// _get_entry_reserved_cost: an entry without a cost of its own was reserved at the reservation's cost
#[must_use]
pub const fn get_entry_reserved_cost(
    entry: &ReservationEntry,
    default_reserved_cost: Decimal,
) -> Decimal {
    let Some(reserved_cost) = entry.reserved_cost else {
        return default_reserved_cost;
    };
    reserved_cost
}
