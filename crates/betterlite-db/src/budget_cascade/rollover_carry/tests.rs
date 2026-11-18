#![cfg(test)]

use std::collections::BTreeMap;

use rust_decimal::Decimal;

use super::rollover_caps;
use crate::budget_cascade::tests::{budget, unnamed_budget};
use crate::budget_reset_policy::rollover::RolloverPolicy;

const ENABLED: RolloverPolicy = RolloverPolicy { enabled: true };
const DISABLED: RolloverPolicy = RolloverPolicy { enabled: false };

#[test]
fn rollover_off_builds_no_caps_even_with_max_budgets() {
    let budgets = vec![
        budget("budget-a", Some(Decimal::from(20_i32)), None),
        budget("budget-b", Some(Decimal::from(5_i32)), None),
    ];
    assert_eq!(rollover_caps(&budgets, DISABLED), BTreeMap::new());
}

#[test]
fn rollover_on_takes_each_tier_max_budget_as_its_cap() {
    let budgets = vec![
        budget("budget-a", Some(Decimal::from(20_i32)), None),
        budget("budget-b", Some(Decimal::from(5_i32)), None),
    ];
    assert_eq!(
        rollover_caps(&budgets, ENABLED),
        BTreeMap::from([
            ("budget-a".to_owned(), Decimal::from(20_i32)),
            ("budget-b".to_owned(), Decimal::from(5_i32)),
        ])
    );
}

#[test]
fn a_tier_without_a_cap_carries_nothing() {
    let budgets = vec![
        budget("budget-a", None, None),
        budget("budget-b", Some(Decimal::from(5_i32)), None),
    ];
    assert_eq!(
        rollover_caps(&budgets, ENABLED),
        BTreeMap::from([("budget-b".to_owned(), Decimal::from(5_i32))])
    );
}

#[test]
fn a_zero_cap_is_a_cap() {
    let budgets = vec![budget("budget-a", Some(Decimal::ZERO), None)];
    assert_eq!(
        rollover_caps(&budgets, ENABLED),
        BTreeMap::from([("budget-a".to_owned(), Decimal::ZERO)])
    );
}

#[test]
fn a_tier_without_an_identifier_is_not_capped() {
    let budgets = vec![
        unnamed_budget(),
        budget("budget-b", Some(Decimal::from(5_i32)), None),
    ];
    assert_eq!(
        rollover_caps(&budgets, ENABLED),
        BTreeMap::from([("budget-b".to_owned(), Decimal::from(5_i32))])
    );
}
