#![cfg(test)]

use rust_decimal::Decimal;

use super::{BaselineCosts, SavingsSpend};

#[test]
fn savings_spend_defaults_every_component_to_zero() {
    let spend = SavingsSpend::default();
    assert_eq!(spend.compression, Decimal::ZERO);
    assert_eq!(spend.prompt_caching, Decimal::ZERO);
    assert_eq!(spend.autorouter, Decimal::ZERO);
    assert_eq!(spend.gateway_injected_caching, Decimal::ZERO);
}

#[test]
fn baseline_costs_savings_is_baseline_minus_actual_unclamped() {
    let costs = BaselineCosts {
        actual: Decimal::new(50, 2),
        baseline: Decimal::new(30, 2),
    };
    assert_eq!(costs.baseline - costs.actual, Decimal::new(-20, 2));
}
