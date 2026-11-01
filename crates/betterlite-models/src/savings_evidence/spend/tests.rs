#![cfg(test)]

use rust_decimal::Decimal;

use super::SavingsSpend;

#[test]
fn savings_spend_defaults_every_component_to_zero() {
    let spend = SavingsSpend::default();
    assert_eq!(spend.compression, Decimal::ZERO);
    assert_eq!(spend.prompt_caching, Decimal::ZERO);
    assert_eq!(spend.autorouter, Decimal::ZERO);
    assert_eq!(spend.gateway_injected_caching, Decimal::ZERO);
}
