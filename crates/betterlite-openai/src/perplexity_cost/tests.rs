#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::cost_usage::{CompletionDetails, CostUsage, PromptDetails};
use betterlite_models::token_cost::GenericCostResult;
use betterlite_prompts::arguments::json::{Json, Object};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::{Value, json};

use super::{PerplexityCostError, PerplexityUsage, cost_per_token};

fn dec(text: &str) -> Decimal {
    text.parse().expect("the decimal fixture")
}

fn moment() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2024-06-16T12:00:00Z")
        .expect("the fixture instant")
        .with_timezone(&Utc)
}

fn database() -> TimeZoneDatabase {
    TimeZoneDatabase::from_env()
}

fn usage(prompt_tokens: i64, completion_tokens: i64) -> PerplexityUsage {
    PerplexityUsage {
        tokens: CostUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens.checked_add(completion_tokens).unwrap_or(0),
            prompt_details: None,
            completion_details: None,
            cache_read_tokens: None,
            numeric_extras: BTreeMap::new(),
        },
        cost: None,
        citation_tokens: None,
        reasoning_tokens: None,
    }
}

fn run(
    model_info: Value,
    usage: &PerplexityUsage,
) -> Result<GenericCostResult, PerplexityCostError> {
    let info: Object = serde_json::from_value(model_info).expect("the model info fixture");
    cost_per_token(&info, usage, moment(), &database())
}

fn cost_value(value: Value) -> Json {
    serde_json::from_value(value).expect("the cost fixture")
}

fn rates() -> Value {
    json!({
        "input_cost_per_token": "0.1",
        "output_cost_per_token": "0.2"
    })
}

#[rstest]
#[case::number(json!(1.25), Ok((0, "1.25")))]
#[case::zero_number(json!(0), Ok((0, "0")))]
#[case::total_in_an_object(json!({"total_cost": 0.5}), Ok((0, "0.5")))]
#[case::zero_total_in_an_object(json!({"total_cost": 0}), Ok((0, "0")))]
#[case::object_without_a_total(json!({"other": 2}), Err("manual"))]
#[case::null_total(json!({"total_cost": null}), Err("manual"))]
#[case::boolean_cost(json!(true), Err("manual"))]
#[case::text_cost(json!("0.5"), Err("manual"))]
fn reported_cost_bypasses_token_pricing(
    #[case] cost: Value,
    #[case] expected: Result<(i64, &str), &str>,
) {
    let mut usage = usage(10, 2);
    usage.cost = Some(serde_json::from_value(cost).expect("the cost fixture"));
    let result = run(rates(), &usage).expect("the reported cost path");
    match expected {
        Ok((prompt, completion)) => {
            assert_eq!(result.prompt_cost, dec(&prompt.to_string()));
            assert_eq!(result.completion_cost, dec(completion));
        }
        Err(_) => {
            assert_eq!(result.prompt_cost, dec("1"));
            assert_eq!(result.completion_cost, dec("0.4"));
        }
    }
}

#[test]
fn an_unparseable_reported_total_is_an_error() {
    let mut usage = usage(10, 2);
    usage.cost = Some(cost_value(json!({"total_cost": "not a price"})));
    let error = run(rates(), &usage).expect_err("the invalid total");
    assert!(matches!(error, PerplexityCostError::ReportedCost));
}

#[test]
fn manual_prompt_and_completion_costs_use_the_token_rates() {
    let result = run(
        json!({
            "input_cost_per_token": "0.000001",
            "output_cost_per_token": "0.000002"
        }),
        &usage(100, 50),
    )
    .expect("the manual path");
    assert_eq!(result.prompt_cost, dec("0.0001"));
    assert_eq!(result.completion_cost, dec("0.0001"));
}

#[test]
fn citation_tokens_add_to_the_prompt_cost() {
    let mut usage = usage(100, 50);
    usage.citation_tokens = Some(8);
    let result = run(
        json!({
            "input_cost_per_token": "0.000001",
            "output_cost_per_token": "0.000002",
            "citation_cost_per_token": "0.0000002"
        }),
        &usage,
    )
    .expect("the citation charge");
    assert_eq!(result.prompt_cost, dec("0.0001016"));
    assert_eq!(result.completion_cost, dec("0.0001"));
}

#[test]
fn a_missing_citation_rate_adds_nothing_to_the_prompt_cost() {
    let mut usage = usage(100, 50);
    usage.citation_tokens = Some(8);
    let result = run(
        json!({
            "input_cost_per_token": "0.000001",
            "output_cost_per_token": "0.000002"
        }),
        &usage,
    )
    .expect("the manual path");
    assert_eq!(result.prompt_cost, dec("0.0001"));
}

#[test]
fn zero_citation_tokens_are_never_charged() {
    let mut usage = usage(100, 50);
    usage.citation_tokens = Some(0);
    let result = run(
        json!({
            "input_cost_per_token": "0.000001",
            "output_cost_per_token": "0.000002",
            "citation_cost_per_token": "0.0000002"
        }),
        &usage,
    )
    .expect("the manual path");
    assert_eq!(result.prompt_cost, dec("0.0001"));
}

#[test]
fn top_level_reasoning_tokens_are_split_at_the_reasoning_rate() {
    let mut usage = usage(10, 10);
    usage.reasoning_tokens = Some(4);
    let result = run(
        json!({
            "input_cost_per_token": "0",
            "output_cost_per_token": "0.5",
            "output_cost_per_reasoning_token": "2"
        }),
        &usage,
    )
    .expect("the reasoning split");
    assert_eq!(result.completion_cost, dec("11"));
}

#[test]
fn reasoning_tokens_fall_back_to_the_completion_details() {
    let mut usage = usage(10, 10);
    usage.tokens.completion_details = Some(CompletionDetails {
        text_tokens: None,
        audio_tokens: None,
        image_tokens: None,
        video_tokens: None,
        reasoning_tokens: Some(4),
    });
    let result = run(
        json!({
            "output_cost_per_token": "0.5",
            "output_cost_per_reasoning_token": "2"
        }),
        &usage,
    )
    .expect("the reasoning split");
    assert_eq!(result.completion_cost, dec("11"));
}

#[test]
fn a_missing_reasoning_rate_charges_every_completion_token_at_the_output_rate() {
    let mut usage = usage(10, 10);
    usage.tokens.completion_details = Some(CompletionDetails {
        text_tokens: None,
        audio_tokens: None,
        image_tokens: None,
        video_tokens: None,
        reasoning_tokens: Some(4),
    });
    let result = run(json!({"output_cost_per_token": "0.5"}), &usage).expect("the output rate");
    assert_eq!(result.completion_cost, dec("5"));
}

#[test]
fn reasoning_tokens_above_the_completion_count_leave_no_plain_tokens() {
    let mut usage = usage(10, 3);
    usage.reasoning_tokens = Some(5);
    let result = run(
        json!({
            "output_cost_per_token": "0.5",
            "output_cost_per_reasoning_token": "2"
        }),
        &usage,
    )
    .expect("the reasoning split");
    assert_eq!(result.completion_cost, dec("10"));
}

#[test]
fn the_search_charge_is_added_to_the_completion_cost() {
    let mut usage = usage(10, 10);
    usage.tokens.prompt_details = Some(PromptDetails {
        text_tokens: None,
        audio_tokens: None,
        image_tokens: None,
        video_tokens: None,
        web_search_requests: Some(2),
    });
    let result = run(
        json!({
            "input_cost_per_token": "0.1",
            "output_cost_per_token": "0.5",
            "search_context_cost_per_query": {"search_context_size_low": "0.005"}
        }),
        &usage,
    )
    .expect("the search charge");
    assert_eq!(result.prompt_cost, dec("1"));
    assert_eq!(result.completion_cost, dec("5.01"));
}

#[test]
fn search_queries_cost_per_query_wins_over_the_context_object() {
    let mut usage = usage(10, 10);
    usage.tokens.prompt_details = Some(PromptDetails {
        text_tokens: None,
        audio_tokens: None,
        image_tokens: None,
        video_tokens: None,
        web_search_requests: Some(3),
    });
    let result = run(
        json!({
            "output_cost_per_token": "0.5",
            "search_queries_cost_per_query": "0.01",
            "search_context_cost_per_query": {"search_context_size_low": "0.005"}
        }),
        &usage,
    )
    .expect("the search charge");
    assert_eq!(result.completion_cost, dec("5.03"));
}

#[test]
fn a_zero_search_queries_rate_falls_through_to_the_context_object() {
    let mut usage = usage(10, 10);
    usage.tokens.prompt_details = Some(PromptDetails {
        text_tokens: None,
        audio_tokens: None,
        image_tokens: None,
        video_tokens: None,
        web_search_requests: Some(3),
    });
    let result = run(
        json!({
            "output_cost_per_token": "0.5",
            "search_queries_cost_per_query": 0,
            "search_context_cost_per_query": {"search_context_size_low": "0.005"}
        }),
        &usage,
    )
    .expect("the search charge");
    assert_eq!(result.completion_cost, dec("5.015"));
}

#[test]
fn a_context_object_without_the_low_size_charges_nothing() {
    let mut usage = usage(10, 10);
    usage.tokens.prompt_details = Some(PromptDetails {
        text_tokens: None,
        audio_tokens: None,
        image_tokens: None,
        video_tokens: None,
        web_search_requests: Some(3),
    });
    let result = run(
        json!({
            "output_cost_per_token": "0.5",
            "search_context_cost_per_query": {"search_context_size_medium": "0.005"}
        }),
        &usage,
    )
    .expect("the search charge");
    assert_eq!(result.completion_cost, dec("5"));
}

#[test]
fn an_unparseable_search_rate_charges_nothing() {
    let mut usage = usage(10, 10);
    usage.tokens.prompt_details = Some(PromptDetails {
        text_tokens: None,
        audio_tokens: None,
        image_tokens: None,
        video_tokens: None,
        web_search_requests: Some(3),
    });
    let result = run(
        json!({
            "output_cost_per_token": "0.5",
            "search_queries_cost_per_query": "not a price"
        }),
        &usage,
    )
    .expect("the search charge");
    assert_eq!(result.completion_cost, dec("5"));
}

#[test]
fn an_absent_search_count_is_never_charged() {
    let result = run(
        json!({
            "output_cost_per_token": "0.5",
            "search_context_cost_per_query": {"search_context_size_low": "0.005"}
        }),
        &usage(10, 10),
    )
    .expect("the manual path");
    assert_eq!(result.completion_cost, dec("5"));
}

#[test]
fn off_peak_rates_replace_the_manual_rates_at_the_supplied_instant() {
    let result = run(
        json!({
            "input_cost_per_token": "0.000001",
            "output_cost_per_token": "0.000002",
            "off_peak_pricing": {
                "hours_utc": "00:00-23:59",
                "input_cost_per_token": "0.5",
                "output_cost_per_token": "1.5"
            }
        }),
        &usage(100, 10),
    )
    .expect("the off-peak rates");
    assert_eq!(result.prompt_cost, dec("50"));
    assert_eq!(result.completion_cost, dec("15"));
}

#[test]
fn off_peak_rates_stay_base_outside_the_window() {
    let result = run(
        json!({
            "input_cost_per_token": "0.5",
            "output_cost_per_token": "1.5",
            "off_peak_pricing": {
                "hours_utc": "00:00-06:00",
                "input_cost_per_token": "1",
                "output_cost_per_token": "2"
            }
        }),
        &usage(100, 10),
    )
    .expect("the base rates");
    assert_eq!(result.prompt_cost, dec("50"));
    assert_eq!(result.completion_cost, dec("15"));
}

#[test]
fn a_missing_off_peak_block_keeps_the_manual_rates() {
    let result = run(
        json!({
            "input_cost_per_token": "0.5",
            "output_cost_per_token": "1.5"
        }),
        &usage(100, 10),
    )
    .expect("the base rates");
    assert_eq!(result.prompt_cost, dec("50"));
    assert_eq!(result.completion_cost, dec("15"));
}

#[test]
fn a_reported_zero_wins_over_every_other_charge() {
    let mut usage = usage(100, 10);
    usage.cost = Some(cost_value(json!(0)));
    usage.citation_tokens = Some(8);
    usage.reasoning_tokens = Some(4);
    let result = run(
        json!({
            "input_cost_per_token": "0.5",
            "output_cost_per_token": "1.5",
            "citation_cost_per_token": "2",
            "output_cost_per_reasoning_token": "3",
            "search_context_cost_per_query": {"search_context_size_low": "0.005"}
        }),
        &usage,
    )
    .expect("the reported cost");
    assert_eq!(result.prompt_cost, Decimal::ZERO);
    assert_eq!(result.completion_cost, Decimal::ZERO);
}
