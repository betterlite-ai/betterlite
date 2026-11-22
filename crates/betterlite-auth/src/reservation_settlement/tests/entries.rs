use rust_decimal::Decimal;

use crate::budget_counter::{BudgetCounter, CounterEntityType};
use crate::reservation_settlement::entry::{counter_to_reservation_entry, get_entry_reserved_cost};

use super::support::{decimal, entry};

#[test]
fn a_counter_becomes_an_entry_reserved_at_the_estimate() {
    let counter = BudgetCounter {
        counter_key: "spend:key:k1".to_owned(),
        max_budget: decimal("100"),
        fallback_spend: Decimal::ZERO,
        entity_type: CounterEntityType::Key,
        entity_id: "k1".to_owned(),
        source_cache_key: None,
        spend_log_entity_id: Some("k1".to_owned()),
        window_duration: None,
        window_start: None,
    };
    let built = counter_to_reservation_entry(&counter, decimal("2.5"));
    assert_eq!(built.counter_key.as_deref(), Some("spend:key:k1"));
    assert_eq!(built.entity_type, CounterEntityType::Key);
    assert_eq!(built.entity_id, "k1");
    assert_eq!(built.reserved_cost, Some(decimal("2.5")));
    assert_eq!(built.applied_adjustment, Decimal::ZERO);
}

#[test]
fn an_entry_without_a_cost_takes_the_reservation_cost() {
    let mut unpriced = entry("spend:key:k1", "10");
    unpriced.reserved_cost = None;
    assert_eq!(
        get_entry_reserved_cost(&unpriced, decimal("7")),
        decimal("7")
    );
}

#[test]
fn a_priced_entry_keeps_its_own_cost() {
    assert_eq!(
        get_entry_reserved_cost(&entry("spend:key:k1", "3"), decimal("7")),
        decimal("3")
    );
}

#[test]
fn an_explicit_zero_cost_stays_zero() {
    assert_eq!(
        get_entry_reserved_cost(&entry("spend:key:k1", "0"), decimal("7")),
        Decimal::ZERO
    );
}
