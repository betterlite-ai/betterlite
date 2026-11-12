#![cfg(test)]

use std::str::FromStr as _;

use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;

use super::web_search_cost;
use crate::cost_usage::server_tool_projection::{ServerToolRecord, ServerToolUse};
use crate::decisions::Json;
use crate::price_fields::nested::SearchContextPricing;
use crate::price_lookup::PriceRecord;

use super::super::error::AnthropicPricingError;

fn decimal(value: &str) -> Decimal {
    Decimal::from_str(value).expect("a fixture decimal parses")
}

fn model_info() -> PriceRecord {
    let mut info = PriceRecord::new();
    info.insert(
        "input_cost_per_token".to_owned(),
        serde_json::from_str("0.25").expect("a fixture scalar parses"),
    );
    info
}

fn pricing(medium: Option<&str>) -> SearchContextPricing {
    SearchContextPricing {
        search_context_size_low: Some(decimal("0.01")),
        search_context_size_medium: medium.map(decimal),
        search_context_size_high: Some(decimal("0.05")),
    }
}

fn mapping(entries: &[(&str, Json)]) -> IndexMap<String, Json> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn count(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn parsed(count: Option<i64>) -> ServerToolUse {
    ServerToolUse {
        web_search_requests: count,
        tool_search_requests: None,
        browser_open_requests: None,
    }
}

#[test]
fn the_medium_rate_multiplies_the_reported_count() {
    let usage = parsed(Some(2));
    let cost = web_search_cost(
        Some(&model_info()),
        Some(&pricing(Some("0.02"))),
        Some(ServerToolRecord::Parsed(&usage)),
    )
    .expect("search pricing succeeds");
    assert_eq!(cost, decimal("0.04"));
}

#[test]
fn negative_counts_keep_their_sign() {
    let usage = parsed(Some(-2));
    let cost = web_search_cost(
        Some(&model_info()),
        Some(&pricing(Some("0.02"))),
        Some(ServerToolRecord::Parsed(&usage)),
    )
    .expect("search pricing succeeds");
    assert_eq!(cost, decimal("-0.04"));
}

#[test]
fn a_reported_zero_costs_nothing() {
    let usage = parsed(Some(0));
    let cost = web_search_cost(
        Some(&model_info()),
        Some(&pricing(Some("0.02"))),
        Some(ServerToolRecord::Parsed(&usage)),
    )
    .expect("search pricing succeeds");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn an_unreported_count_costs_nothing() {
    let cost = web_search_cost(
        Some(&model_info()),
        Some(&pricing(Some("0.02"))),
        Some(ServerToolRecord::Parsed(&parsed(None))),
    )
    .expect("search pricing succeeds");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn a_mapping_count_is_read_through_the_tolerant_accessor() {
    let record = mapping(&[("web_search_requests", count(2))]);
    let cost = web_search_cost(
        Some(&model_info()),
        Some(&pricing(Some("0.02"))),
        Some(ServerToolRecord::Mapping(&record)),
    )
    .expect("search pricing succeeds");
    assert_eq!(cost, decimal("0.04"));
}

#[test]
fn a_boolean_mapping_count_counts_as_one() {
    let record = mapping(&[("web_search_requests", Json::Bool(true))]);
    let cost = web_search_cost(
        Some(&model_info()),
        Some(&pricing(Some("0.02"))),
        Some(ServerToolRecord::Mapping(&record)),
    )
    .expect("search pricing succeeds");
    assert_eq!(cost, decimal("0.02"));
}

#[test]
fn a_non_numeric_mapping_count_is_a_typed_failure() {
    let record = mapping(&[("web_search_requests", Json::Text("3".to_owned()))]);
    let result = web_search_cost(
        Some(&model_info()),
        Some(&pricing(Some("0.02"))),
        Some(ServerToolRecord::Mapping(&record)),
    );
    assert!(matches!(
        result,
        Err(AnthropicPricingError::InvalidSearchCount { .. })
    ));
}

#[test]
fn missing_model_info_costs_nothing_even_with_a_count() {
    let usage = parsed(Some(2));
    let cost = web_search_cost(
        None,
        Some(&pricing(Some("0.02"))),
        Some(ServerToolRecord::Parsed(&usage)),
    )
    .expect("search pricing succeeds");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn missing_usage_costs_nothing() {
    let cost = web_search_cost(Some(&model_info()), Some(&pricing(Some("0.02"))), None)
        .expect("search pricing succeeds");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn missing_search_pricing_costs_nothing() {
    let usage = parsed(Some(2));
    let cost = web_search_cost(
        Some(&model_info()),
        None,
        Some(ServerToolRecord::Parsed(&usage)),
    )
    .expect("search pricing succeeds");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn a_missing_medium_rate_costs_nothing() {
    let usage = parsed(Some(2));
    let cost = web_search_cost(
        Some(&model_info()),
        Some(&pricing(None)),
        Some(ServerToolRecord::Parsed(&usage)),
    )
    .expect("search pricing succeeds");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn a_zero_medium_rate_costs_nothing() {
    let usage = parsed(Some(2));
    let cost = web_search_cost(
        Some(&model_info()),
        Some(&pricing(Some("0"))),
        Some(ServerToolRecord::Parsed(&usage)),
    )
    .expect("search pricing succeeds");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn a_non_numeric_count_is_not_read_when_the_rate_is_missing() {
    let record = mapping(&[("web_search_requests", Json::Text("3".to_owned()))]);
    let result = web_search_cost(
        Some(&model_info()),
        None,
        Some(ServerToolRecord::Mapping(&record)),
    )
    .expect("search pricing succeeds");
    assert_eq!(result, Decimal::ZERO);
}

#[test]
fn multiplication_overflow_is_a_typed_failure() {
    let usage = parsed(Some(2));
    let huge = SearchContextPricing {
        search_context_size_low: None,
        search_context_size_medium: Some(Decimal::MAX),
        search_context_size_high: None,
    };
    let result = web_search_cost(
        Some(&model_info()),
        Some(&huge),
        Some(ServerToolRecord::Parsed(&usage)),
    );
    assert!(matches!(result, Err(AnthropicPricingError::Overflow)));
}
