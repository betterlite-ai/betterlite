#![cfg(test)]

use betterlite_prometheus::collector_factory::collector::MetricValue;

use super::{RemainingBudget, remaining_budget};
use crate::prometheus::budget_gauges::test_support::decimal;

#[test]
fn no_max_budget_is_unlimited() {
    assert_eq!(
        remaining_budget(None, Some(decimal("4"))),
        RemainingBudget::Unlimited
    );
}

#[test]
fn no_spend_keeps_the_whole_limit() {
    assert_eq!(
        remaining_budget(Some(decimal("10")), None),
        RemainingBudget::Amount(decimal("10"))
    );
}

#[test]
fn spend_is_subtracted_from_the_limit() {
    assert_eq!(
        remaining_budget(Some(decimal("10")), Some(decimal("4"))),
        RemainingBudget::Amount(decimal("6"))
    );
}

#[test]
fn overspend_leaves_a_negative_amount() {
    assert_eq!(
        remaining_budget(Some(decimal("2")), Some(decimal("5"))),
        RemainingBudget::Amount(decimal("-3"))
    );
}

#[test]
fn unlimited_is_written_as_positive_infinity() {
    assert_eq!(
        RemainingBudget::Unlimited.metric_value(),
        MetricValue::PositiveInfinity
    );
}

#[test]
fn an_amount_is_written_as_its_decimal() {
    assert_eq!(
        RemainingBudget::Amount(decimal("6")).metric_value(),
        MetricValue::Finite(decimal("6"))
    );
}
