#![cfg(test)]

use super::{
    Scalar, Tier, TieredPricingError, coerce_cost_per_token, select_tier_for_input, tier_rate,
};
use rstest::rstest;
use rust_decimal::Decimal;
use std::ptr;
use std::str::FromStr as _;

fn tier(range: &str, rate: &str) -> Tier {
    let source = format!(r#"{{"range":{range},"input_cost_per_token":{rate}}}"#);
    serde_json::from_str(&source).expect("a fixture tier parses")
}

fn number(text: &str) -> Scalar {
    serde_json::from_str(text).expect("a fixture number parses")
}

fn selected_is<'tier>(selected: Option<&'tier Tier>, expected: &'tier Tier) -> bool {
    selected.is_some_and(|tier| ptr::eq(tier, expected))
}

#[rstest]
#[case::null(None, "0")]
#[case::number(Some(number("0.0000001")), "0.0000001")]
#[case::scientific_number(Some(number("4e-7")), "0.0000004")]
#[case::scientific_string(Some(number("\"4e-7\"")), "0.0000004")]
#[case::true_boolean(Some(number("true")), "1")]
#[case::false_boolean(Some(number("false")), "0")]
#[case::invalid_string(Some(number("\"not-a-number\"")), "0")]
#[case::incomplete_exponent(Some(number("\"5e\"")), "0")]
#[case::underscored_fraction(Some(number("\"0.000_003\"")), "0.000003")]
#[case::underscored_integer(Some(number("\"1_000\"")), "1000")]
#[case::doubled_underscore(Some(number("\"1__0\"")), "0")]
#[case::leading_underscore(Some(number("\"_1\"")), "0")]
fn coerce_cost_per_token_tolerates_loose_scalars(
    #[case] value: Option<Scalar>,
    #[case] expected: &str,
) {
    let coerced = coerce_cost_per_token(value.as_ref()).expect("a tolerant scalar coerces");
    assert_eq!(coerced, Decimal::from_str(expected).expect("valid decimal"));
}

#[rstest]
#[case::array("[1]")]
#[case::object(r#"{"rate":1}"#)]
fn coerce_cost_per_token_rejects_unsupported_shapes(#[case] text: &str) {
    coerce_cost_per_token(Some(&number(text))).unwrap_err();
}

#[test]
fn coerce_cost_per_token_defaults_absent_value_to_zero() {
    assert_eq!(
        coerce_cost_per_token(None).expect("an absent value coerces"),
        Decimal::ZERO
    );
}

#[test]
fn tier_rate_prefers_a_present_primary_including_explicit_zero() {
    let primary = number("0");
    let fallback = number("5");
    let rate = tier_rate(Some(&primary), Some(&fallback)).expect("primary coerces");
    assert_eq!(rate, Decimal::ZERO);
}

#[test]
fn tier_rate_prefers_an_invalid_primary_string_that_coerces_to_zero() {
    let primary = number("\"not-a-number\"");
    let fallback = number("5");
    let rate = tier_rate(Some(&primary), Some(&fallback)).expect("primary coerces");
    assert_eq!(rate, Decimal::ZERO);
}

#[test]
fn tier_rate_uses_the_fallback_when_the_primary_is_null() {
    let fallback = number("5");
    let rate = tier_rate(None, Some(&fallback)).expect("fallback coerces");
    assert_eq!(rate, Decimal::from_str("5").expect("valid decimal"));
}

#[test]
fn tier_rate_uses_fallback_for_a_null_scalar() {
    let fallback = number("5");
    let rate = tier_rate(Some(&Scalar::Null), Some(&fallback)).expect("fallback coerces");
    assert_eq!(rate, Decimal::from(5));
}

#[test]
fn tier_rate_defaults_to_zero_when_both_fields_are_absent() {
    let rate = tier_rate(None, None).expect("fallback coerces");
    assert_eq!(rate, Decimal::ZERO);
}

#[test]
fn select_tier_for_input_returns_none_for_an_empty_list_or_nonpositive_tokens() {
    let tiers = vec![tier("[0,100]", "1")];
    assert!(
        select_tier_for_input(&[], 50)
            .expect("empty selects")
            .is_none()
    );
    assert!(
        select_tier_for_input(&tiers, 0)
            .expect("zero selects")
            .is_none()
    );
    assert!(
        select_tier_for_input(&tiers, -1)
            .expect("negative selects")
            .is_none()
    );
}

#[test]
fn select_tier_for_input_matches_exclusive_lower_and_inclusive_upper() {
    let tiers = vec![tier("[0,100]", "1"), tier("[100,200]", "2")];
    let at_zero = select_tier_for_input(&tiers, 0).expect("selection succeeds");
    assert!(at_zero.is_none());
    let at_boundary = select_tier_for_input(&tiers, 100).expect("selection succeeds");
    assert!(selected_is(at_boundary, &tiers[0]));
    let past_boundary = select_tier_for_input(&tiers, 101).expect("selection succeeds");
    assert!(selected_is(past_boundary, &tiers[1]));
}

#[test]
fn select_tier_for_input_falls_back_to_the_last_tier_in_a_gap_and_above_every_range() {
    let tiers = vec![tier("[0,100]", "1"), tier("[200,300]", "2")];
    let in_gap = select_tier_for_input(&tiers, 150).expect("selection succeeds");
    assert!(selected_is(in_gap, &tiers[1]));
    let above_every_range = select_tier_for_input(&tiers, 1000).expect("selection succeeds");
    assert!(selected_is(above_every_range, &tiers[1]));
}

#[test]
fn select_tier_for_input_is_stable_for_tiers_with_the_same_first_endpoint() {
    let tiers = vec![tier("[0,100]", "1"), tier("[0,50]", "2")];
    let selected = select_tier_for_input(&tiers, 30).expect("selection succeeds");
    assert!(selected_is(selected, &tiers[0]));
}

#[test]
fn select_tier_for_input_retains_only_two_element_ranges() {
    let tiers = vec![
        tier("[0]", "1"),
        tier("[0,100,200]", "2"),
        tier("[0,300]", "3"),
    ];
    let selected = select_tier_for_input(&tiers, 30).expect("selection succeeds");
    assert!(selected_is(selected, &tiers[2]));
}

#[test]
fn select_tier_for_input_returns_none_when_no_valid_length_range_remains() {
    let tiers = vec![tier("[0]", "1")];
    let selected = select_tier_for_input(&tiers, 30).expect("selection succeeds");
    assert!(selected.is_none());
}

#[test]
fn select_tier_for_input_errors_on_an_empty_range_during_the_sort_before_the_length_filter() {
    let tiers = vec![tier("[]", "1"), tier("[0,300]", "2")];
    select_tier_for_input(&tiers, 30).unwrap_err();
}

#[test]
fn select_tier_for_input_errors_on_a_range_endpoint_that_is_not_a_number() {
    let tiers = vec![tier(r#"[0,"100"]"#, "1")];
    assert!(matches!(
        select_tier_for_input(&tiers, 30),
        Err(TieredPricingError::MalformedRange)
    ));
}

#[test]
fn select_tier_for_input_errors_on_an_explicitly_null_range_during_sort() {
    let tiers = vec![tier("null", "1"), tier("[0,300]", "2")];
    assert!(matches!(
        select_tier_for_input(&tiers, 30),
        Err(TieredPricingError::MalformedRange)
    ));
}

#[test]
fn select_tier_for_input_errors_on_a_null_range_endpoint() {
    let tiers = vec![tier("[0,null]", "1")];
    assert!(matches!(
        select_tier_for_input(&tiers, 30),
        Err(TieredPricingError::MalformedRange)
    ));
}

#[test]
fn select_tier_for_input_evaluates_every_matching_candidate() {
    let tiers = vec![tier("[0,100]", "1"), tier("[10,null]", "2")];
    assert!(matches!(
        select_tier_for_input(&tiers, 50),
        Err(TieredPricingError::MalformedRange)
    ));
}

#[test]
fn select_tier_for_input_falls_back_without_reading_a_later_upper_endpoint() {
    let tiers = vec![tier("[0,100]", "1"), tier("[200,null]", "2")];
    let fallback = select_tier_for_input(&tiers, 150).expect("selection succeeds");
    assert!(selected_is(fallback, &tiers[1]));
}

#[test]
fn select_tier_for_input_coerces_boolean_range_endpoints_like_python() {
    let tiers = vec![tier("[false,true]", "1")];
    let selected = select_tier_for_input(&tiers, 1).expect("selection succeeds");
    assert!(selected_is(selected, &tiers[0]));
}

#[test]
fn select_tier_for_input_propagates_an_endpoint_exact_decimal_cannot_hold() {
    let tiers = vec![tier("[0,1e30]", "1")];
    assert!(matches!(
        select_tier_for_input(&tiers, 30),
        Err(TieredPricingError::Rate(_))
    ));
}

#[test]
fn coerce_cost_per_token_errors_on_a_string_that_overflows_exact_decimal() {
    let overflowing = number("\"1e999999999999999999999\"");
    coerce_cost_per_token(Some(&overflowing)).unwrap_err();
}

#[test]
fn coerce_cost_per_token_errors_on_a_string_with_too_many_significant_digits() {
    let too_precise = number("\"99999999999999999999999999999999\"");
    coerce_cost_per_token(Some(&too_precise)).unwrap_err();
}

#[test]
fn coerce_cost_per_token_errors_on_a_string_float_accepts_but_decimal_cannot_represent() {
    let infinite = number("\"inf\"");
    coerce_cost_per_token(Some(&infinite)).unwrap_err();
}

#[test]
fn tier_deserialization_rejects_an_unmodeled_key_by_name() {
    let error = serde_json::from_str::<Tier>(r#"{"not_a_real_field":1}"#)
        .expect_err("an unmodeled key fails deserialization");
    assert!(error.to_string().contains("not_a_real_field"));
}

#[test]
fn select_tier_for_input_defaults_a_missing_range_to_zero_for_sorting() {
    let without_range: Tier = serde_json::from_str(r#"{"input_cost_per_token":1}"#)
        .expect("a fixture tier without a range parses");
    let tiers = vec![without_range, tier("[0,100]", "2")];
    let selected = select_tier_for_input(&tiers, 50).expect("selection succeeds");
    assert!(selected_is(selected, &tiers[1]));
}
