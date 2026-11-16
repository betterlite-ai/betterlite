#![cfg(test)]

use std::collections::BTreeMap;

use rust_decimal::Decimal;

use super::{
    UsageUnitKey, by, first_match, row_tracked_cost, row_untracked_units, sum_counter_units,
    sum_tracked_cost, sum_untracked_units,
};
use crate::guardrails::usage_projections::records::UsageCounterRow;

fn unit(
    counter: &str,
    guardrail: &str,
    units: i64,
    untracked_units: i64,
    cost: Option<Decimal>,
) -> UsageCounterRow {
    UsageCounterRow {
        usage_unit: counter.to_owned(),
        guardrail_id: guardrail.to_owned(),
        team_id: "team-a".to_owned(),
        api_key: "key-1".to_owned(),
        units,
        untracked_units,
        cost,
    }
}

fn price(units: i64) -> UsageCounterRow {
    unit("requests", "lakera", units, 0, Some(Decimal::new(50, 2)))
}

fn refs(rows: &[UsageCounterRow]) -> Vec<&UsageCounterRow> {
    rows.iter().collect()
}

#[test]
fn sum_counter_units_adds_rows_of_the_same_counter() {
    let rows = [price(4), price(6), unit("tokens", "lakera", 9, 9, None)];
    let sums = sum_counter_units(&refs(&rows));
    assert_eq!(sums.get("requests"), Some(&10));
    assert_eq!(sums.get("tokens"), Some(&9));
    assert_eq!(sums.len(), 2);
}

#[test]
fn sum_untracked_units_falls_back_to_units_without_a_cost() {
    let rows = [
        unit("requests", "lakera", 4, 1, Some(Decimal::new(50, 2))),
        unit("requests", "lakera", 6, 0, None),
    ];
    let sums = sum_untracked_units(&refs(&rows));
    assert_eq!(sums.get("requests"), Some(&7));
}

#[test]
fn sum_untracked_units_omits_counters_that_are_fully_priced() {
    let rows = [
        unit("requests", "lakera", 4, 0, Some(Decimal::new(50, 2))),
        unit("tokens", "lakera", 8, 3, Some(Decimal::new(50, 2))),
    ];
    let sums = sum_untracked_units(&refs(&rows));
    assert_eq!(sums.get("tokens"), Some(&3));
    assert_eq!(sums.get("requests"), None);
}

#[test]
fn sum_tracked_cost_skips_rows_with_no_price() {
    let rows = [
        unit("requests", "lakera", 4, 0, Some(Decimal::new(125, 2))),
        unit("requests", "lakera", 6, 6, None),
        unit("requests", "lakera", 2, 0, Some(Decimal::new(50, 2))),
    ];
    assert_eq!(sum_tracked_cost(&refs(&rows)), Some(Decimal::new(175, 2)));
}

#[test]
fn sum_tracked_cost_skips_rows_whose_units_are_all_untracked() {
    let rows = [
        unit("requests", "lakera", 4, 4, Some(Decimal::new(125, 2))),
        unit("requests", "lakera", 4, 5, Some(Decimal::new(125, 2))),
    ];
    assert_eq!(sum_tracked_cost(&refs(&rows)), None);
}

#[test]
fn sum_tracked_cost_is_null_when_no_row_is_priced() {
    let rows = [unit("requests", "lakera", 4, 0, None)];
    assert_eq!(sum_tracked_cost(&refs(&rows)), None);
    assert_eq!(sum_tracked_cost(&[]), None);
}

#[test]
fn row_tracked_cost_keeps_a_row_that_prices_one_unit() {
    let row = unit("requests", "lakera", 2, 1, Some(Decimal::new(125, 2)));
    assert_eq!(row_tracked_cost(&row), Some(Decimal::new(125, 2)));
}

#[test]
fn row_untracked_units_reads_the_column_when_the_row_has_a_cost() {
    let tracked = unit("requests", "lakera", 10, 4, Some(Decimal::new(125, 2)));
    let legacy = unit("requests", "lakera", 10, 4, None);
    assert_eq!(row_untracked_units(&tracked), 4);
    assert_eq!(row_untracked_units(&legacy), 10);
}

#[test]
fn grouping_by_each_column_splits_the_sums() {
    let mut presidio = unit("requests", "presidio", 6, 0, Some(Decimal::new(50, 2)));
    presidio.team_id = "team-b".to_owned();
    let mut tokens = unit("tokens", "lakera", 2, 0, Some(Decimal::new(50, 2)));
    tokens.api_key = "key-2".to_owned();
    let rows = [
        unit("requests", "lakera", 4, 0, Some(Decimal::new(50, 2))),
        presidio,
        tokens,
    ];
    let by_counter = by(&rows, UsageUnitKey::Counter, sum_counter_units);
    let by_guardrail = by(&rows, UsageUnitKey::Guardrail, sum_counter_units);
    let by_team = by(&rows, UsageUnitKey::Team, sum_counter_units);
    let by_key = by(&rows, UsageUnitKey::Key, sum_counter_units);
    assert_eq!(
        by_counter.get("requests"),
        Some(&BTreeMap::from([("requests".to_owned(), 10)]))
    );
    assert_eq!(
        by_guardrail.get("lakera"),
        Some(&BTreeMap::from([
            ("requests".to_owned(), 4),
            ("tokens".to_owned(), 2)
        ]))
    );
    assert_eq!(
        by_team.get("team-b"),
        Some(&BTreeMap::from([("requests".to_owned(), 6)]))
    );
    assert_eq!(
        by_key.get("key-1"),
        Some(&BTreeMap::from([("requests".to_owned(), 10)]))
    );
}

#[test]
fn first_match_takes_the_first_lookup_key_the_mapping_holds() {
    let mapping = BTreeMap::from([("id-1".to_owned(), 7)]);
    assert_eq!(first_match(&["name", "id-1"], &mapping, &0), &7);
    assert_eq!(first_match(&["name"], &mapping, &0), &0);
    assert_eq!(first_match(&[], &mapping, &0), &0);
}
