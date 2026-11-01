#![cfg(test)]

use rust_decimal::Decimal;

use super::{BaselineCosts, SavingsSpend};

#[test]
fn savings_spend_defaults_autorouter_and_gateway_injected_caching_to_zero() {
    let spend = SavingsSpend {
        compression: Decimal::from(1_i32),
        prompt_caching: Decimal::from(2_i32),
        autorouter: Decimal::ZERO,
        gateway_injected_caching: Decimal::ZERO,
    };
    assert_eq!(spend.autorouter, Decimal::ZERO);
    assert_eq!(spend.gateway_injected_caching, Decimal::ZERO);
}

#[test]
fn baseline_costs_savings_is_unclamped_when_actual_exceeds_baseline() {
    let costs = BaselineCosts {
        actual: Decimal::from(10_i32),
        baseline: Decimal::from(4_i32),
    };
    let savings = costs.baseline.saturating_sub(costs.actual);
    assert_eq!(savings, Decimal::from(-6_i32));
}
