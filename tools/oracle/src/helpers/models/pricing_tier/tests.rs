#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;
use rstest::rstest;
use serde_json::json;

fn input(tiers: &str, tokens: i64, cost_key: &str, fallback_cost_key: Option<&str>) -> Input {
    let source = json!({
        "tiers": serde_json::from_str::<serde_json::Value>(tiers).expect("fixture tiers parse"),
        "tokens": tokens,
        "cost_key": cost_key,
        "fallback_cost_key": fallback_cost_key,
    });
    serde_json::from_value(source).expect("a fixture input parses")
}

#[rstest]
#[case::at_lower_bound(0_i64, "0", "0")]
#[case::past_lower_bound(1_i64, "0", "100")]
#[case::at_upper_bound(100_i64, "0", "100")]
#[case::past_upper_bound(101_i64, "100", "200")]
fn selection_respects_exclusive_lower_inclusive_upper(
    #[case] tokens: i64,
    #[case] lower: &str,
    #[case] upper: &str,
) {
    let tiers = r#"[{"range":[0,100],"input_cost_per_token":1},{"range":[100,200],"input_cost_per_token":2}]"#;
    let request = input(tiers, tokens, "input_cost_per_token", None);
    let output = Op::run(&request).expect("selection succeeds");
    if tokens == 0 {
        assert_eq!(output["selected_range"], serde_json::Value::Null);
        return;
    }
    let lower_value: serde_json::Value = serde_json::from_str(lower).expect("lower parses");
    let upper_value: serde_json::Value = serde_json::from_str(upper).expect("upper parses");
    assert_eq!(output["selected_range"], json!([lower_value, upper_value]));
}

#[test]
fn a_gap_and_a_point_above_every_range_both_fall_back_to_the_last_tier() {
    let tiers = r#"[{"range":[0,100],"input_cost_per_token":1},{"range":[200,300],"input_cost_per_token":2}]"#;
    for tokens in [150_i64, 1000_i64] {
        let request = input(tiers, tokens, "input_cost_per_token", None);
        let output = Op::run(&request).expect("selection succeeds");
        assert_eq!(output["selected_range"], json!([200_i64, 300_i64]));
        assert_eq!(output["rate"], "2");
    }
}

#[test]
fn ties_on_the_first_endpoint_keep_the_original_order() {
    let tiers = r#"[{"range":[0,100],"input_cost_per_token":1},{"range":[0,50],"input_cost_per_token":2}]"#;
    let request = input(tiers, 30, "input_cost_per_token", None);
    let output = Op::run(&request).expect("selection succeeds");
    assert_eq!(output["selected_range"], json!([0_i64, 100_i64]));
    assert_eq!(output["rate"], "1");
}

#[test]
fn an_empty_range_errors_during_the_sort_before_the_length_filter_can_drop_it() {
    let tiers = r#"[{"range":[]},{"range":[0,300],"input_cost_per_token":1}]"#;
    let request = input(tiers, 30, "input_cost_per_token", None);
    let error = Op::run(&request).expect_err("a malformed range errors");
    assert_eq!(error.class, "TieredPricingError");
}

#[rstest]
#[case::explicit_zero_primary_suppresses_fallback("0")]
#[case::invalid_string_primary_also_suppresses_fallback(r#""not-a-number""#)]
fn a_present_primary_wins_over_the_fallback(#[case] primary_literal: &str) {
    let tiers = format!(
        r#"[{{"range":[0,100],"input_cost_per_token":{primary_literal},"output_cost_per_token":5}}]"#
    );
    let request = input(
        &tiers,
        50,
        "input_cost_per_token",
        Some("output_cost_per_token"),
    );
    let output = Op::run(&request).expect("selection and rate succeed");
    assert_eq!(output["rate"], "0");
}

#[test]
fn a_null_primary_uses_the_fallback() {
    let tiers = r#"[{"range":[0,100],"output_cost_per_token":5}]"#;
    let request = input(
        tiers,
        50,
        "input_cost_per_token",
        Some("output_cost_per_token"),
    );
    let output = Op::run(&request).expect("selection and rate succeed");
    assert_eq!(output["rate"], "5");
}

#[rstest]
#[case::scientific_string(r#""4e-7""#, "0.0000004")]
#[case::boolean_true("true", "1")]
fn the_rate_tolerates_scientific_strings_and_booleans(
    #[case] rate_literal: &str,
    #[case] expected: &str,
) {
    let tiers = format!(r#"[{{"range":[0,100],"input_cost_per_token":{rate_literal}}}]"#);
    let request = input(&tiers, 50, "input_cost_per_token", None);
    let output = Op::run(&request).expect("selection and rate succeed");
    assert_eq!(output["rate"], expected);
}
