#![cfg(test)]

use super::{CostBreakdown, CostBreakdownError, cost_breakdown_with_guardrail};
use rust_decimal::Decimal;
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
    };
    let unchanged = cost_breakdown_with_guardrail(Some(&existing), Decimal::ZERO)
        .expect("a nonpositive cost never overflows")
        .expect("the existing breakdown passes through");
    assert_eq!(unchanged, existing);

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
fn cost_breakdown_with_guardrail_preserves_every_prior_field_and_adds_to_the_total() {
    let existing = CostBreakdown {
        service_tier: Some("default".to_owned()),
        data_residency: None,
        vertex_location: None,
        input_cost: Some(decimal("1")),
        cache_read_cost: None,
        cache_creation_cost: None,
        output_cost: Some(decimal("2")),
        reasoning_cost: None,
        total_cost: Some(decimal("3")),
        tool_usage_cost: None,
        guardrail_cost: None,
        additional_costs: None,
        original_cost: None,
        discount_percent: None,
        discount_amount: None,
        margin_percent: None,
        margin_fixed_amount: None,
        margin_total_amount: None,
    };
    let merged = cost_breakdown_with_guardrail(Some(&existing), decimal("0.5"))
        .expect("the addition succeeds")
        .expect("a positive cost always produces a breakdown");
    assert_eq!(merged.input_cost, existing.input_cost);
    assert_eq!(merged.output_cost, existing.output_cost);
    assert_eq!(merged.service_tier, existing.service_tier);
    assert_eq!(merged.guardrail_cost, Some(decimal("0.5")));
    assert_eq!(merged.total_cost, Some(decimal("3.5")));
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
    };
    let error = cost_breakdown_with_guardrail(Some(&existing), decimal("1"))
        .expect_err("the addition overflows Decimal");
    assert!(matches!(error, CostBreakdownError::GuardrailCostOverflow));
}

#[test]
fn cost_breakdown_deserializes_an_empty_object_to_every_field_absent() {
    let empty: CostBreakdown = serde_json::from_str("{}").expect("an empty object parses");
    assert_eq!(empty, CostBreakdown::default());
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
