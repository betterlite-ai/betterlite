#![cfg(test)]

use std::str::FromStr as _;

use rust_decimal::Decimal;

use super::{apply_surcharges, speed_multiplier};
use crate::cost_usage::server_tool_projection::ServerToolProjection;
use crate::price_lookup::PriceRecord;
use crate::token_cost::GenericCostResult;

fn decimal(value: &str) -> Decimal {
    Decimal::from_str(value).expect("a fixture decimal parses")
}

fn entry(pairs: &[(&str, &str)]) -> PriceRecord {
    pairs
        .iter()
        .map(|(key, value)| {
            (
                (*key).to_owned(),
                serde_json::from_str(value).expect("a fixture scalar parses"),
            )
        })
        .collect()
}

fn projection(inference_geo: Option<&str>, speed: Option<&str>) -> ServerToolProjection {
    ServerToolProjection {
        server_tool_use: None,
        inference_geo: inference_geo.map(str::to_owned),
        speed: speed.map(str::to_owned),
    }
}

fn costs(prompt: &str, completion: &str) -> GenericCostResult {
    GenericCostResult {
        prompt_cost: decimal(prompt),
        completion_cost: decimal(completion),
    }
}

fn run(entry: Option<&PriceRecord>, projection: &ServerToolProjection) -> GenericCostResult {
    let mut costs = costs("1", "1");
    apply_surcharges(entry, projection, &mut costs);
    costs
}

#[test]
fn fast_speed_and_geo_multiply_the_generic_totals() {
    let entry = entry(&[("us", "1.1"), ("fast", "2")]);
    let result = run(Some(&entry), &projection(Some("us"), Some("fast")));
    assert_eq!(result.prompt_cost, decimal("2.2"));
    assert_eq!(result.completion_cost, decimal("2.2"));
}

#[test]
fn geo_multiplies_without_a_fast_speed() {
    let entry = entry(&[("us", "1.1")]);
    let result = run(Some(&entry), &projection(Some("us"), None));
    assert_eq!(result.prompt_cost, decimal("1.1"));
    assert_eq!(result.completion_cost, decimal("1.1"));
}

#[rstest::rstest]
#[case::exact_fast("fast", "2")]
#[case::capitalized("Fast", "1")]
#[case::upper_case("FAST", "1")]
#[case::empty("", "1")]
fn speed_needs_the_exact_fast_string(#[case] speed: &str, #[case] expected: &str) {
    let entry = entry(&[("fast", "2")]);
    let result = run(Some(&entry), &projection(None, Some(speed)));
    assert_eq!(result.prompt_cost, decimal(expected));
    assert_eq!(result.completion_cost, decimal(expected));
}

#[test]
fn an_absent_speed_leaves_the_totals_alone() {
    let entry = entry(&[("fast", "2")]);
    let result = run(Some(&entry), &projection(None, None));
    assert_eq!(result.prompt_cost, Decimal::ONE);
    assert_eq!(result.completion_cost, Decimal::ONE);
}

#[test]
fn an_empty_entry_leaves_the_totals_alone() {
    let entry = entry(&[]);
    let result = run(Some(&entry), &projection(Some("us"), Some("fast")));
    assert_eq!(result.prompt_cost, Decimal::ONE);
    assert_eq!(result.completion_cost, Decimal::ONE);
}

#[test]
fn an_absent_entry_leaves_the_totals_alone() {
    let result = run(None, &projection(Some("us"), Some("fast")));
    assert_eq!(result.prompt_cost, Decimal::ONE);
    assert_eq!(result.completion_cost, Decimal::ONE);
}

#[test]
fn an_invalid_geo_price_aborts_before_the_speed_surcharge() {
    let entry = entry(&[("us", "\"bad\""), ("fast", "2")]);
    let result = run(Some(&entry), &projection(Some("us"), Some("fast")));
    assert_eq!(result.prompt_cost, Decimal::ONE);
    assert_eq!(result.completion_cost, Decimal::ONE);
}

#[test]
fn a_null_geo_price_aborts_before_the_speed_surcharge() {
    let entry = entry(&[("us", "null"), ("fast", "2")]);
    let result = run(Some(&entry), &projection(Some("us"), Some("fast")));
    assert_eq!(result.prompt_cost, Decimal::ONE);
    assert_eq!(result.completion_cost, Decimal::ONE);
}

#[rstest::rstest]
#[case::global("global")]
#[case::not_available("not_available")]
#[case::upper_case_global("GLOBAL")]
fn a_non_regional_geo_still_applies_the_speed_surcharge(#[case] geo: &str) {
    let entry = entry(&[("us", "1.1"), ("fast", "2")]);
    let result = run(Some(&entry), &projection(Some(geo), Some("fast")));
    assert_eq!(result.prompt_cost, decimal("2"));
    assert_eq!(result.completion_cost, decimal("2"));
}

#[test]
fn a_numeric_string_fast_price_aborts_the_geo_surcharge() {
    let entry = entry(&[("us", "1.1"), ("fast", "\"2\"")]);
    let result = run(Some(&entry), &projection(Some("us"), Some("fast")));
    assert_eq!(result.prompt_cost, Decimal::ONE);
    assert_eq!(result.completion_cost, Decimal::ONE);
}

#[test]
fn a_null_fast_price_aborts_the_geo_surcharge() {
    let entry = entry(&[("us", "1.1"), ("fast", "null")]);
    let result = run(Some(&entry), &projection(Some("us"), Some("fast")));
    assert_eq!(result.prompt_cost, Decimal::ONE);
    assert_eq!(result.completion_cost, Decimal::ONE);
}

#[test]
fn a_zero_fast_price_zeroes_the_totals() {
    let entry = entry(&[("fast", "0")]);
    let result = run(Some(&entry), &projection(None, Some("fast")));
    assert_eq!(result.prompt_cost, Decimal::ZERO);
    assert_eq!(result.completion_cost, Decimal::ZERO);
}

#[test]
fn a_false_fast_price_zeroes_the_totals() {
    let entry = entry(&[("fast", "false")]);
    let result = run(Some(&entry), &projection(None, Some("fast")));
    assert_eq!(result.prompt_cost, Decimal::ZERO);
    assert_eq!(result.completion_cost, Decimal::ZERO);
}

#[test]
fn a_true_fast_price_leaves_the_totals_alone() {
    let entry = entry(&[("fast", "true")]);
    let result = run(Some(&entry), &projection(None, Some("fast")));
    assert_eq!(result.prompt_cost, Decimal::ONE);
    assert_eq!(result.completion_cost, Decimal::ONE);
}

#[test]
fn the_speed_scaled_amount_survives_a_later_geo_overflow() {
    let huge = Decimal::MAX.to_string();
    let entry = entry(&[("us", huge.as_str()), ("fast", "2")]);
    let result = run(Some(&entry), &projection(Some("us"), Some("fast")));
    assert_eq!(result.prompt_cost, decimal("2"));
    assert_eq!(result.completion_cost, decimal("2"));
}

#[test]
fn speed_multiplier_reads_numeric_and_boolean_values_only() {
    let numeric = entry(&[("fast", "2.5")]);
    assert_eq!(
        speed_multiplier(&numeric, Some("fast")),
        Some(decimal("2.5"))
    );
    let boolean = entry(&[("fast", "true")]);
    assert_eq!(speed_multiplier(&boolean, Some("fast")), Some(Decimal::ONE));
    let list = entry(&[("fast", "[]")]);
    assert_eq!(speed_multiplier(&list, Some("fast")), None);
    let object = entry(&[("fast", "{}")]);
    assert_eq!(speed_multiplier(&object, Some("fast")), None);
}
