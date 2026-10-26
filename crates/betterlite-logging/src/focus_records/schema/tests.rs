#![cfg(test)]

use super::{ColumnType, DECIMAL_PRECISION, DECIMAL_SCALE, FOCUS_NORMALIZED_SCHEMA};
use rstest::rstest;

#[test]
fn empty_schema_has_37_columns_in_litellm_order() {
    let names: Vec<&str> = FOCUS_NORMALIZED_SCHEMA
        .iter()
        .map(|(name, _)| *name)
        .collect();
    assert_eq!(names.len(), 37);
    assert_eq!(names.first(), Some(&"BilledCost"));
    assert_eq!(names.get(4), Some(&"BillingPeriodStart"));
    assert_eq!(names.get(5), Some(&"BillingPeriodEnd"));
    assert_eq!(names.last(), Some(&"Tags"));
}

#[rstest]
#[case::billed_cost("BilledCost", ColumnType::Decimal)]
#[case::billing_period_start("BillingPeriodStart", ColumnType::Timestamp)]
#[case::charge_period_end("ChargePeriodEnd", ColumnType::Timestamp)]
#[case::contracted_unit_price("ContractedUnitPrice", ColumnType::Decimal)]
#[case::pricing_quantity("PricingQuantity", ColumnType::Decimal)]
#[case::region_id("RegionId", ColumnType::Text)]
#[case::tags_hold_json_text("Tags", ColumnType::Text)]
fn columns_keep_litellm_types(#[case] name: &str, #[case] expected: ColumnType) {
    let found = FOCUS_NORMALIZED_SCHEMA
        .iter()
        .find(|(column, _)| *column == name)
        .map(|(_, kind)| *kind);
    assert_eq!(found, Some(expected));
}

#[test]
fn decimal_is_eighteen_digits_with_six_after_the_point() {
    assert_eq!((DECIMAL_PRECISION, DECIMAL_SCALE), (18, 6));
    let decimals = FOCUS_NORMALIZED_SCHEMA
        .iter()
        .filter(|(_, kind)| *kind == ColumnType::Decimal)
        .count();
    assert_eq!(decimals, 8);
}
