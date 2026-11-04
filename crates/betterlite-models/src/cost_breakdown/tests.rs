#![cfg(test)]

use super::{CostBreakdown, CostBreakdownError, cost_breakdown_with_guardrail};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::str::FromStr as _;

fn decimal(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a fixture decimal parses")
}

#[test]
fn cost_breakdown_with_guardrail_passes_through_unchanged_for_a_nonpositive_cost() {
    let existing = CostBreakdown {
        service_tier: None,
        data_residency: None,
        vertex_location: None,
        input_cost: Some(decimal("1")),
        cache_read_cost: None,
        cache_creation_cost: None,
        output_cost: None,
        reasoning_cost: None,
        total_cost: None,
        tool_usage_cost: None,
        guardrail_cost: None,
        additional_costs: None,
        original_cost: None,
        discount_percent: None,
        discount_amount: None,
        margin_percent: None,
        margin_fixed_amount: None,
        margin_total_amount: None,
        extra: BTreeMap::new(),
    };
    let unchanged = cost_breakdown_with_guardrail(Some(&existing), Decimal::ZERO)
        .expect("a nonpositive cost never overflows")
        .expect("the existing breakdown passes through");
    assert_eq!(
        serde_json::to_string(&unchanged).expect("the breakdown serializes"),
        serde_json::to_string(&existing).expect("the breakdown serializes")
    );

    let still_none = cost_breakdown_with_guardrail(None, Decimal::ZERO)
        .expect("a nonpositive cost never overflows");
    assert!(still_none.is_none());
}

#[test]
fn cost_breakdown_with_guardrail_starts_empty_when_no_breakdown_exists() {
    let merged = cost_breakdown_with_guardrail(None, decimal("2.5"))
        .expect("the addition succeeds")
        .expect("a positive cost always produces a breakdown");
    assert_eq!(merged.guardrail_cost, Some(decimal("2.5")));
    assert_eq!(merged.total_cost, Some(decimal("2.5")));
    assert_eq!(merged.input_cost, None);
}

#[test]
fn cost_breakdown_with_guardrail_changes_only_guardrail_and_total_cost() {
    let existing = CostBreakdown {
        service_tier: Some("default".to_owned()),
        data_residency: Some("eu".to_owned()),
        vertex_location: Some("us-central1".to_owned()),
        input_cost: Some(decimal("1")),
        cache_read_cost: Some(decimal("0.5")),
        cache_creation_cost: Some(decimal("0.25")),
        output_cost: Some(decimal("2")),
        reasoning_cost: Some(decimal("0.25")),
        total_cost: Some(decimal("3")),
        tool_usage_cost: Some(decimal("0.25")),
        guardrail_cost: Some(decimal("0.25")),
        additional_costs: Some(BTreeMap::from([("other".to_owned(), decimal("0.25"))])),
        original_cost: Some(decimal("3")),
        discount_percent: Some(decimal("0.25")),
        discount_amount: Some(decimal("0.25")),
        margin_percent: Some(decimal("0.25")),
        margin_fixed_amount: Some(decimal("0.25")),
        margin_total_amount: Some(decimal("0.25")),
        extra: BTreeMap::from([("vendor_extra".to_owned(), decimal("0.25"))]),
    };
    let merged = cost_breakdown_with_guardrail(Some(&existing), decimal("0.5"))
        .expect("the addition succeeds")
        .expect("a positive cost always produces a breakdown");
    let mut expected = existing;
    expected.guardrail_cost = Some(decimal("0.5"));
    expected.total_cost = Some(decimal("3.5"));
    assert_eq!(merged, expected);
}

#[test]
fn cost_breakdown_with_guardrail_adds_again_on_repeated_calls_without_subtracting_the_prior_amount()
{
    let existing = CostBreakdown {
        service_tier: None,
        data_residency: None,
        vertex_location: None,
        input_cost: None,
        cache_read_cost: None,
        cache_creation_cost: None,
        output_cost: None,
        reasoning_cost: None,
        total_cost: Some(decimal("10")),
        tool_usage_cost: None,
        guardrail_cost: Some(decimal("1")),
        additional_costs: None,
        original_cost: None,
        discount_percent: None,
        discount_amount: None,
        margin_percent: None,
        margin_fixed_amount: None,
        margin_total_amount: None,
        extra: BTreeMap::new(),
    };
    let merged = cost_breakdown_with_guardrail(Some(&existing), decimal("1"))
        .expect("the addition succeeds")
        .expect("a positive cost always produces a breakdown");
    assert_eq!(merged.guardrail_cost, Some(decimal("1")));
    assert_eq!(merged.total_cost, Some(decimal("11")));
}

#[test]
fn cost_breakdown_with_guardrail_treats_a_missing_total_cost_as_zero() {
    let existing = CostBreakdown {
        service_tier: None,
        data_residency: None,
        vertex_location: None,
        input_cost: Some(decimal("1")),
        cache_read_cost: None,
        cache_creation_cost: None,
        output_cost: None,
        reasoning_cost: None,
        total_cost: None,
        tool_usage_cost: None,
        guardrail_cost: None,
        additional_costs: None,
        original_cost: None,
        discount_percent: None,
        discount_amount: None,
        margin_percent: None,
        margin_fixed_amount: None,
        margin_total_amount: None,
        extra: BTreeMap::new(),
    };
    let merged = cost_breakdown_with_guardrail(Some(&existing), decimal("4"))
        .expect("the addition succeeds")
        .expect("a positive cost always produces a breakdown");
    assert_eq!(merged.total_cost, Some(decimal("4")));
}

#[test]
fn cost_breakdown_with_guardrail_propagates_a_total_cost_overflow() {
    let existing = CostBreakdown {
        service_tier: None,
        data_residency: None,
        vertex_location: None,
        input_cost: None,
        cache_read_cost: None,
        cache_creation_cost: None,
        output_cost: None,
        reasoning_cost: None,
        total_cost: Some(decimal("79228162514264337593543950335")),
        tool_usage_cost: None,
        guardrail_cost: None,
        additional_costs: None,
        original_cost: None,
        discount_percent: None,
        discount_amount: None,
        margin_percent: None,
        margin_fixed_amount: None,
        margin_total_amount: None,
        extra: BTreeMap::new(),
    };
    let error = cost_breakdown_with_guardrail(Some(&existing), decimal("1"))
        .expect_err("the addition overflows Decimal");
    assert!(matches!(error, CostBreakdownError::GuardrailCostOverflow));
}

#[test]
fn cost_breakdown_with_guardrail_keeps_an_unknown_extension_field() {
    let existing: CostBreakdown =
        serde_json::from_str(r#"{"input_cost": 1, "total_cost": 1, "cache_write_cost": 0.25}"#)
            .expect("a breakdown with an unknown key parses");
    let merged = cost_breakdown_with_guardrail(Some(&existing), decimal("0.5"))
        .expect("the addition succeeds")
        .expect("a positive cost always produces a breakdown");
    assert_eq!(merged.extra["cache_write_cost"], decimal("0.25"));
    assert_eq!(merged.input_cost, existing.input_cost);
    assert_eq!(merged.guardrail_cost, Some(decimal("0.5")));
    assert_eq!(merged.total_cost, Some(decimal("1.5")));
}

#[test]
fn cost_breakdown_serializes_money_as_json_numbers() {
    #[derive(serde::Deserialize)]
    struct NumericBreakdown {
        input_cost: serde_json::Number,
        additional_costs: BTreeMap<String, serde_json::Number>,
        vendor_extra: serde_json::Number,
    }

    let breakdown = CostBreakdown {
        service_tier: None,
        data_residency: None,
        vertex_location: None,
        input_cost: Some(decimal("1")),
        cache_read_cost: None,
        cache_creation_cost: None,
        output_cost: None,
        reasoning_cost: None,
        total_cost: None,
        tool_usage_cost: None,
        guardrail_cost: None,
        additional_costs: Some(BTreeMap::from([("other".to_owned(), decimal("0.5"))])),
        original_cost: None,
        discount_percent: None,
        discount_amount: None,
        margin_percent: None,
        margin_fixed_amount: None,
        margin_total_amount: None,
        extra: BTreeMap::from([("vendor_extra".to_owned(), decimal("0.25"))]),
    };
    let text = serde_json::to_string(&breakdown).expect("the breakdown serializes");
    let value: NumericBreakdown =
        serde_json::from_str(&text).expect("the breakdown contains JSON numbers");
    assert_eq!(value.input_cost.to_string(), "1");
    assert_eq!(value.additional_costs["other"].to_string(), "0.5");
    assert_eq!(value.vendor_extra.to_string(), "0.25");
}

#[test]
fn cost_breakdown_deserializes_an_empty_object_to_every_field_absent() {
    let empty: CostBreakdown = serde_json::from_str("{}").expect("an empty object parses");
    assert_eq!(
        serde_json::to_string(&empty).expect("the breakdown serializes"),
        "{}"
    );
}

#[test]
fn cost_breakdown_deserializes_exact_decimals_including_scientific_notation() {
    let parsed: CostBreakdown =
        serde_json::from_str(r#"{"input_cost": 4e-7, "additional_costs": {"flat": 1.5}}"#)
            .expect("a fixture breakdown parses");
    assert_eq!(parsed.input_cost, Some(decimal("0.0000004")));
    assert_eq!(
        parsed.additional_costs.expect("the map is present")["flat"],
        decimal("1.5")
    );
}
