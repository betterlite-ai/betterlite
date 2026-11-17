#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;

use super::{CostEstimateRequest, CostEstimateResponse};
use crate::key_fields::values::Amount;
use crate::proxy_request_contracts::tests::{assert_decode, assert_validation, contract};

const CACHE_SUBSET: &str =
    "cache_read_input_tokens plus cache_creation_input_tokens cannot exceed input_tokens";
const REASONING_SUBSET: &str = "reasoning_tokens cannot exceed output_tokens";

#[test]
fn defaults_the_optional_counts() {
    let request = CostEstimateRequest::from_value(&contract(
        r#"{"model": "gpt-4o", "input_tokens": 10, "output_tokens": 5}"#,
    ))
    .expect("the required counts");
    assert_eq!(request.cache_read_input_tokens, 0);
    assert_eq!(request.cache_creation_input_tokens, 0);
    assert_eq!(request.reasoning_tokens, 0);
    assert_eq!(request.num_requests_per_day, None);
    assert_eq!(request.num_requests_per_month, None);
}

#[rstest]
#[case::input_tokens(
    r#"{"model": "m", "input_tokens": -1, "output_tokens": 5}"#,
    "input_tokens"
)]
#[case::output_tokens(
    r#"{"model": "m", "input_tokens": 10, "output_tokens": -1}"#,
    "output_tokens"
)]
#[case::cache_read(
    r#"{"model": "m", "input_tokens": 10, "output_tokens": 5, "cache_read_input_tokens": -1}"#,
    "cache_read_input_tokens"
)]
#[case::cache_creation(
    r#"{"model": "m", "input_tokens": 10, "output_tokens": 5, "cache_creation_input_tokens": -1}"#,
    "cache_creation_input_tokens"
)]
#[case::reasoning(
    r#"{"model": "m", "input_tokens": 10, "output_tokens": 5, "reasoning_tokens": -1}"#,
    "reasoning_tokens"
)]
#[case::requests_per_day(
    r#"{"model": "m", "input_tokens": 10, "output_tokens": 5, "num_requests_per_day": -1}"#,
    "num_requests_per_day"
)]
#[case::requests_per_month(
    r#"{"model": "m", "input_tokens": 10, "output_tokens": 5, "num_requests_per_month": -1}"#,
    "num_requests_per_month"
)]
fn rejects_a_negative_count(#[case] text: &str, #[case] field: &str) {
    let error = CostEstimateRequest::from_value(&contract(text)).expect_err("a count below zero");
    assert_validation(&error, &format!("{field} must be at least 0"));
}

#[rstest]
#[case::one_over(r#"{"model": "m", "input_tokens": 10, "output_tokens": 5, "cache_read_input_tokens": 6, "cache_creation_input_tokens": 5}"#)]
#[case::cache_only(
    r#"{"model": "m", "input_tokens": 10, "output_tokens": 5, "cache_read_input_tokens": 11}"#
)]
fn rejects_cache_counts_above_the_input(#[case] text: &str) {
    let error = CostEstimateRequest::from_value(&contract(text)).expect_err("more cache tokens");
    assert_validation(&error, CACHE_SUBSET);
}

#[test]
fn rejects_reasoning_tokens_above_the_output() {
    let error = CostEstimateRequest::from_value(&contract(
        r#"{"model": "m", "input_tokens": 10, "output_tokens": 5, "reasoning_tokens": 6}"#,
    ))
    .expect_err("more reasoning tokens");
    assert_validation(&error, REASONING_SUBSET);
}

#[test]
fn reports_the_cache_message_when_both_subsets_break() {
    let error = CostEstimateRequest::from_value(&contract(
        r#"{
            "model": "m",
            "input_tokens": 10,
            "output_tokens": 5,
            "cache_read_input_tokens": 11,
            "reasoning_tokens": 6
        }"#,
    ))
    .expect_err("both subsets break");
    assert_validation(&error, CACHE_SUBSET);
}

#[rstest]
#[case::cache_reaches_input(r#"{"model": "m", "input_tokens": 10, "output_tokens": 5, "cache_read_input_tokens": 6, "cache_creation_input_tokens": 4}"#)]
#[case::reasoning_reaches_output(
    r#"{"model": "m", "input_tokens": 10, "output_tokens": 5, "reasoning_tokens": 5}"#
)]
#[case::no_cache(r#"{"model": "m", "input_tokens": 0, "output_tokens": 0}"#)]
fn accepts_counts_equal_to_the_totals(#[case] text: &str) {
    CostEstimateRequest::from_value(&contract(text)).expect("a subset that reaches the total");
}

#[rstest]
#[case::model("{}")]
#[case::input_tokens(r#"{"model": "m", "output_tokens": 5}"#)]
#[case::output_tokens(r#"{"model": "m", "input_tokens": 10}"#)]
fn requires_the_model_and_counts(#[case] text: &str) {
    let error = CostEstimateRequest::from_value(&contract(text)).expect_err("a required field");
    assert_decode(error);
}

#[test]
fn response_defaults_its_costs() {
    let response = CostEstimateResponse::from_value(&contract(
        r#"{
            "model": "gpt-4o",
            "input_tokens": 10,
            "output_tokens": 5,
            "cost_per_request": 0.5,
            "input_cost_per_request": 0.4,
            "output_cost_per_request": 0.1
        }"#,
    ))
    .expect("the required costs");
    assert_eq!(response.cost_per_request, Amount(Decimal::new(5, 1)));
    assert_eq!(response.margin_cost_per_request, Amount(Decimal::ZERO));
    assert_eq!(response.cache_read_cost_per_request, Amount(Decimal::ZERO));
    assert_eq!(response.cache_creation_cost, Amount(Decimal::ZERO));
    assert_eq!(response.reasoning_cost_per_request, Amount(Decimal::ZERO));
    assert_eq!(response.daily_cost, None);
    assert_eq!(response.monthly_cost, None);
    assert_eq!(response.input_cost_per_token, None);
    assert_eq!(response.provider, None);
}

#[test]
fn serializes_the_response_in_declaration_order() {
    let response = CostEstimateResponse::from_value(&contract(
        r#"{
            "model": "gpt-4o",
            "input_tokens": 10,
            "output_tokens": 5,
            "cost_per_request": 0.5,
            "input_cost_per_request": 0.4,
            "output_cost_per_request": 0.1
        }"#,
    ))
    .expect("the required costs");
    let text = serde_json::to_string(&response).expect("the dump");
    assert!(text.starts_with(
        r#"{"model":"gpt-4o","input_tokens":10,"output_tokens":5,"cache_read_input_tokens":0"#
    ));
    assert!(text.ends_with(r#""output_cost_per_reasoning_token":null,"provider":null}"#));
}
