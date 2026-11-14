use std::collections::BTreeMap;

use betterlite_models::guardrail_pricing::{GuardrailPricingError, azure_prompt_shield_cost};
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::usage_with;
use crate::azure_prompt_shield::billing::{BillingUsage, billing_tracing_detail};
use crate::azure_prompt_shield::cost::CostTier;

#[test]
fn a_free_tier_costs_zero() {
    let detail = billing_tracing_detail(usage_with(&["hello"]), Some(CostTier::Free), None)
        .unwrap()
        .unwrap();
    assert_eq!(detail.guardrail_cost, Some(Decimal::ZERO));
    assert_eq!(detail.guardrail_cost_in_spend, Some(false));
}

#[test]
fn a_free_tier_ignores_a_configured_price() {
    let detail = billing_tracing_detail(
        usage_with(&["hello"]),
        Some(CostTier::Free),
        Some(Decimal::new(2, 0)),
    )
    .unwrap()
    .unwrap();
    assert_eq!(detail.guardrail_cost, Some(Decimal::ZERO));
}

#[test]
fn a_missing_price_tracks_usage_without_a_cost() {
    let detail = billing_tracing_detail(usage_with(&["hello"]), None, None)
        .unwrap()
        .unwrap();
    assert_eq!(detail.guardrail_cost, None);
    assert_eq!(detail.guardrail_cost_in_spend, None);
    assert_eq!(detail.guardrail_usage.as_ref().map(BTreeMap::len), Some(3));
}

#[test]
fn a_paid_tier_multiplies_the_text_records_exactly() {
    let long = "x".repeat(3331);
    let detail = billing_tracing_detail(
        usage_with(&[long.as_str()]),
        Some(CostTier::Paid),
        Some(Decimal::new(21, 4)),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        detail.guardrail_usage.as_ref().unwrap().get("text_records"),
        Some(&4)
    );
    assert_eq!(detail.guardrail_cost, Some(Decimal::new(84, 7)));
    assert_eq!(detail.guardrail_cost_in_spend, Some(false));
}

#[rstest]
#[case::unpriced(None, None)]
#[case::free_tier(Some(CostTier::Free), None)]
#[case::paid_without_a_count(Some(CostTier::Paid), Some(Decimal::new(5, 1)))]
#[case::unpriced_with_a_price(None, Some(Decimal::new(5, 1)))]
fn a_missing_text_record_count_bills_zero(
    #[case] cost_tier: Option<CostTier>,
    #[case] price: Option<Decimal>,
) {
    let units = BTreeMap::from([("requests".to_owned(), 3_i64)]);
    let cost = azure_prompt_shield_cost(&units, cost_tier.map(CostTier::wire_name), price).unwrap();
    let expected = (cost_tier == Some(CostTier::Free) || price.is_some()).then_some(Decimal::ZERO);
    assert_eq!(cost, expected);
}

#[test]
fn an_empty_usage_has_no_detail() {
    assert_eq!(
        billing_tracing_detail(BillingUsage::default(), None, None).unwrap(),
        None
    );
}

#[test]
fn an_overflowing_cost_is_an_error() {
    let long = "x".repeat(3000);
    let error = billing_tracing_detail(
        usage_with(&[long.as_str()]),
        Some(CostTier::Paid),
        Some(Decimal::MAX),
    )
    .unwrap_err();
    assert!(
        matches!(&error, GuardrailPricingError::CostOverflow),
        "{error}"
    );
}
