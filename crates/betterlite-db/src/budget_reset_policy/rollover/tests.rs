#![cfg(test)]

use super::{RolloverPolicy, carried_spend, reset_spend, row_carried_spend};
use rstest::rstest;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

const ENABLED: RolloverPolicy = RolloverPolicy { enabled: true };
const DISABLED: RolloverPolicy = RolloverPolicy { enabled: false };

#[rstest]
#[case::over_the_cap_carries_the_overage(
    ENABLED,
    Some(Decimal::from(150_i32)),
    Some(Decimal::from(100_i32)),
    Decimal::from(50_i32)
)]
#[case::under_the_cap_carries_nothing(
    ENABLED,
    Some(Decimal::from(40_i32)),
    Some(Decimal::from(100_i32)),
    Decimal::ZERO
)]
#[case::exactly_at_the_cap_carries_nothing(
    ENABLED,
    Some(Decimal::from(100_i32)),
    Some(Decimal::from(100_i32)),
    Decimal::ZERO
)]
#[case::zero_cap_carries_everything(
    ENABLED,
    Some(Decimal::from(150_i32)),
    Some(Decimal::ZERO),
    Decimal::from(150_i32)
)]
#[case::no_cap_carries_nothing(ENABLED, Some(Decimal::from(150_i32)), None, Decimal::ZERO)]
#[case::no_spend_carries_nothing(ENABLED, None, Some(Decimal::from(100_i32)), Decimal::ZERO)]
#[case::disabled_carries_nothing(
    DISABLED,
    Some(Decimal::from(150_i32)),
    Some(Decimal::from(100_i32)),
    Decimal::ZERO
)]
#[case::disabled_even_with_a_zero_cap_carries_nothing(
    DISABLED,
    Some(Decimal::from(150_i32)),
    Some(Decimal::ZERO),
    Decimal::ZERO
)]
fn rollover_reset_spend(
    #[case] policy: RolloverPolicy,
    #[case] spend: Option<Decimal>,
    #[case] cap: Option<Decimal>,
    #[case] expected: Decimal,
) {
    assert_eq!(reset_spend(policy, spend, cap), expected);
}

#[test]
fn rollover_policy_defaults_to_disabled() {
    assert!(!RolloverPolicy::default().enabled);
}

#[test]
fn rollover_carried_spend_without_a_cap_is_zero() {
    assert_eq!(
        carried_spend(Some(Decimal::from(150_i32)), None),
        Decimal::ZERO
    );
}

#[test]
fn rollover_carried_spend_with_a_zero_cap_is_all_of_it() {
    assert_eq!(
        carried_spend(Some(Decimal::from(150_i32)), Some(Decimal::ZERO)),
        Decimal::from(150_i32)
    );
}

#[test]
fn rollover_carried_spend_under_the_cap_is_zero() {
    assert_eq!(
        carried_spend(Some(Decimal::from(40_i32)), Some(Decimal::from(100_i32))),
        Decimal::ZERO
    );
}

#[test]
fn rollover_carried_spend_without_spend_is_zero() {
    assert_eq!(
        carried_spend(None, Some(Decimal::from(100_i32))),
        Decimal::ZERO
    );
}

fn caps(entries: [(&str, Decimal); 1]) -> BTreeMap<String, Decimal> {
    entries
        .into_iter()
        .map(|(budget_id, cap)| (budget_id.to_owned(), cap))
        .collect()
}

#[test]
fn rollover_row_carried_spend_uses_the_rows_tier_cap() {
    assert_eq!(
        row_carried_spend(
            Some(Decimal::from(150_i32)),
            Some("budget-a"),
            &caps([("budget-a", Decimal::from(100_i32))])
        ),
        Decimal::from(50_i32)
    );
}

#[test]
fn rollover_row_carried_spend_without_a_linked_budget_is_zero() {
    assert_eq!(
        row_carried_spend(
            Some(Decimal::from(150_i32)),
            None,
            &caps([("budget-a", Decimal::from(100_i32))])
        ),
        Decimal::ZERO
    );
}

#[test]
fn rollover_row_carried_spend_for_an_unknown_tier_is_zero() {
    assert_eq!(
        row_carried_spend(
            Some(Decimal::from(150_i32)),
            Some("budget-b"),
            &caps([("budget-a", Decimal::from(100_i32))])
        ),
        Decimal::ZERO
    );
}

#[test]
fn rollover_row_carried_spend_without_any_caps_is_zero() {
    assert_eq!(
        row_carried_spend(
            Some(Decimal::from(150_i32)),
            Some("budget-a"),
            &BTreeMap::new()
        ),
        Decimal::ZERO
    );
}
