#![cfg(test)]

use betterlite_models::cost_usage::CostUsage;
use betterlite_models::off_peak::schema::OffPeakBlockEntry;
use betterlite_models::price_lookup::PricedModel;
use betterlite_models::tiered_pricing::Tier;
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use betterlite_models::token_rates::ModelInfo;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use std::str::FromStr as _;

use super::{CostRequest, cost_per_token};

fn decimal(value: &str) -> Decimal {
    Decimal::from_str(value).expect("a test decimal")
}

fn priced(source: &str) -> PricedModel {
    let value: serde_json::Value = serde_json::from_str(source).expect("the model info JSON");
    let record: ModelInfo = serde_json::from_value(value.clone()).expect("the price record");
    let tiered_pricing: Vec<Tier> = value.get("tiered_pricing").map_or_else(Vec::new, |tiers| {
        serde_json::from_value(tiers.clone()).expect("the tiers")
    });
    let off_peak_pricing = value
        .get("off_peak_pricing")
        .and_then(|block| serde_json::from_value::<OffPeakBlockEntry>(block.clone()).ok())
        .and_then(|entry| entry.as_block().cloned());
    PricedModel {
        record,
        tiered_pricing,
        off_peak_pricing,
        off_peak_cache_read_present: false,
    }
}

fn usage(source: &str) -> CostUsage {
    serde_json::from_str(source).expect("the usage")
}

fn extras(cache_creation_tokens: Option<i64>) -> PromptExtras {
    PromptExtras {
        cache_write_tokens: None,
        cache_creation_tokens,
        cache_creation_token_details: None,
        cached_tokens_details: None,
        character_count: None,
        image_count: None,
        video_length_seconds: None,
        audio_length_seconds: None,
        query_count: None,
    }
}

fn moment(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("a test instant")
        .with_timezone(&Utc)
}

fn cost(
    model_info: &PricedModel,
    usage: &CostUsage,
    extras: &PromptExtras,
    current_time: DateTime<Utc>,
) -> (Decimal, Decimal) {
    let request = CostRequest {
        model_info,
        usage,
        prompt_extras: extras,
        current_time,
    };
    let result = cost_per_token(&request, &TimeZoneDatabase::from_env()).expect("the cost");
    (result.prompt_cost, result.completion_cost)
}

// 85 text tokens, 10 cached, 5 cache-creation; 30 text completion tokens and 20 reasoning tokens
fn split_usage(prompt_tokens: i64) -> CostUsage {
    usage(&format!(
        r#"{{"prompt_tokens":{prompt_tokens},"completion_tokens":50,"cache_read_tokens":10,"prompt_details":{{"text_tokens":85}},"completion_details":{{"reasoning_tokens":20}}}}"#
    ))
}

#[test]
fn dashscope_cost_bills_each_token_class_at_its_own_flat_rate() {
    let model = priced(
        r#"{"input_cost_per_token":1,"output_cost_per_token":2,"cache_read_input_token_cost":0,"cache_creation_input_token_cost":0.5,"output_cost_per_reasoning_token":0.25}"#,
    );
    let (prompt, completion) = cost(
        &model,
        &split_usage(100),
        &extras(Some(5)),
        moment("2024-06-16T12:00:00Z"),
    );
    assert_eq!(prompt, decimal("87.5"));
    assert_eq!(completion, decimal("65"));
}

#[test]
fn dashscope_cost_falls_back_to_the_input_rate_only_for_absent_cache_rates() {
    let model = priced(r#"{"input_cost_per_token":1,"output_cost_per_token":2}"#);
    let (prompt, completion) = cost(
        &model,
        &split_usage(100),
        &extras(Some(5)),
        moment("2024-06-16T12:00:00Z"),
    );
    assert_eq!(prompt, decimal("100"));
    assert_eq!(completion, decimal("100"));
}

#[test]
fn dashscope_cost_keeps_a_present_zero_cache_rate_free() {
    let model = priced(
        r#"{"input_cost_per_token":1,"output_cost_per_token":2,"cache_read_input_token_cost":0,"cache_creation_input_token_cost":0}"#,
    );
    let (prompt, completion) = cost(
        &model,
        &split_usage(100),
        &extras(Some(5)),
        moment("2024-06-16T12:00:00Z"),
    );
    assert_eq!(prompt, decimal("85"));
    assert_eq!(completion, decimal("100"));
}

#[test]
fn dashscope_cost_selects_one_tier_for_the_whole_request_by_total_input() {
    let model = priced(
        r#"{"input_cost_per_token":9,"output_cost_per_token":9,"tiered_pricing":[
            {"range":[0,100],"input_cost_per_token":1,"output_cost_per_token":2,"cache_read_input_token_cost":0,"cache_creation_input_token_cost":0},
            {"range":[100,100000],"input_cost_per_token":3,"output_cost_per_token":4,"output_cost_per_reasoning_token":5,"cache_read_input_token_cost":0.5,"cache_creation_input_token_cost":0.25}
        ]}"#,
    );
    let at_boundary = cost(
        &model,
        &split_usage(100),
        &extras(Some(5)),
        moment("2024-06-16T12:00:00Z"),
    );
    assert_eq!(at_boundary, (decimal("85"), decimal("100")));
    let above = cost(
        &model,
        &split_usage(150),
        &extras(Some(5)),
        moment("2024-06-16T12:00:00Z"),
    );
    assert_eq!(above, (decimal("411.25"), decimal("220")));
}

#[test]
fn dashscope_cost_tier_without_output_rate_keeps_the_flat_output_rate() {
    let model = priced(
        r#"{"input_cost_per_token":9,"output_cost_per_token":7,"output_cost_per_reasoning_token":0.5,"tiered_pricing":[
            {"range":[0,100000],"input_cost_per_token":1}
        ]}"#,
    );
    let (prompt, completion) = cost(
        &model,
        &split_usage(100),
        &extras(Some(5)),
        moment("2024-06-16T12:00:00Z"),
    );
    assert_eq!(prompt, decimal("100"));
    assert_eq!(completion, decimal("220"));
}

#[test]
fn dashscope_cost_tier_declaring_output_bills_reasoning_at_its_output_rate() {
    let model = priced(
        r#"{"input_cost_per_token":9,"output_cost_per_token":7,"output_cost_per_reasoning_token":0.5,"tiered_pricing":[
            {"range":[0,100000],"input_cost_per_token":1,"output_cost_per_token":4}
        ]}"#,
    );
    let (_, completion) = cost(
        &model,
        &split_usage(100),
        &extras(Some(5)),
        moment("2024-06-16T12:00:00Z"),
    );
    assert_eq!(completion, decimal("200"));
}

#[test]
fn dashscope_cost_tier_declaring_reasoning_uses_it() {
    let model = priced(
        r#"{"input_cost_per_token":9,"output_cost_per_token":7,"tiered_pricing":[
            {"range":[0,100000],"input_cost_per_token":1,"output_cost_per_token":4,"output_cost_per_reasoning_token":5}
        ]}"#,
    );
    let (_, completion) = cost(
        &model,
        &split_usage(100),
        &extras(Some(5)),
        moment("2024-06-16T12:00:00Z"),
    );
    assert_eq!(completion, decimal("220"));
}

#[test]
fn dashscope_cost_applies_tier_rates_above_every_declared_range() {
    let model = priced(
        r#"{"input_cost_per_token":9,"output_cost_per_token":9,"tiered_pricing":[
            {"range":[0,10],"input_cost_per_token":1,"output_cost_per_token":2},
            {"range":[10,20],"input_cost_per_token":3,"output_cost_per_token":4}
        ]}"#,
    );
    let (prompt, completion) = cost(
        &model,
        &split_usage(100),
        &extras(Some(5)),
        moment("2024-06-16T12:00:00Z"),
    );
    assert_eq!(prompt, decimal("300"));
    assert_eq!(completion, decimal("200"));
}

#[test]
fn dashscope_cost_off_peak_rates_replace_the_standard_rates_while_the_window_is_open() {
    let model = priced(
        r#"{"input_cost_per_token":1,"output_cost_per_token":2,"off_peak_pricing":{"hours_utc":"11:00-13:00","input_cost_per_token":10,"output_cost_per_token":20}}"#,
    );
    let usage = usage(
        r#"{"prompt_tokens":10,"completion_tokens":5,"completion_details":{"reasoning_tokens":2}}"#,
    );
    let open = cost(
        &model,
        &usage,
        &PromptExtras::default(),
        moment("2024-06-16T12:00:00Z"),
    );
    assert_eq!(open, (decimal("100"), decimal("100")));
    let closed = cost(
        &model,
        &usage,
        &PromptExtras::default(),
        moment("2024-06-16T14:00:00Z"),
    );
    assert_eq!(closed, (decimal("10"), decimal("10")));
}

#[test]
fn dashscope_cost_off_peak_without_reasoning_rate_bills_reasoning_at_the_off_peak_output() {
    let model = priced(
        r#"{"input_cost_per_token":1,"output_cost_per_token":2,"tiered_pricing":[
            {"range":[0,100000],"input_cost_per_token":1,"output_cost_per_token":4}
        ],"off_peak_pricing":{"hours_utc":"11:00-13:00","output_cost_per_token":20}}"#,
    );
    let usage = usage(
        r#"{"prompt_tokens":10,"completion_tokens":5,"completion_details":{"reasoning_tokens":2}}"#,
    );
    let (_, completion) = cost(
        &model,
        &usage,
        &PromptExtras::default(),
        moment("2024-06-16T12:00:00Z"),
    );
    assert_eq!(completion, decimal("100"));
}

#[test]
fn dashscope_cost_treats_missing_completion_tokens_as_zero() {
    let model = priced(r#"{"input_cost_per_token":1,"output_cost_per_token":2}"#);
    let usage = usage(r#"{"prompt_tokens":10}"#);
    let (prompt, completion) = cost(
        &model,
        &usage,
        &PromptExtras::default(),
        moment("2024-06-16T12:00:00Z"),
    );
    assert_eq!(prompt, decimal("10"));
    assert_eq!(completion, Decimal::ZERO);
}

#[test]
fn dashscope_cost_clamps_a_cache_overlap_to_zero_text_tokens() {
    let model = priced(
        r#"{"input_cost_per_token":1,"output_cost_per_token":2,"cache_read_input_token_cost":0.5,"cache_creation_input_token_cost":0.25}"#,
    );
    let usage = usage(
        r#"{"prompt_tokens":5,"completion_tokens":0,"cache_read_tokens":10,"prompt_details":{"text_tokens":0}}"#,
    );
    let (prompt, completion) = cost(
        &model,
        &usage,
        &extras(Some(5)),
        moment("2024-06-16T12:00:00Z"),
    );
    assert_eq!(prompt, decimal("6.25"));
    assert_eq!(completion, Decimal::ZERO);
}

#[test]
fn dashscope_cost_clamps_reasoning_above_completion_tokens_to_zero() {
    let model = priced(
        r#"{"input_cost_per_token":1,"output_cost_per_token":2,"output_cost_per_reasoning_token":3}"#,
    );
    let usage = usage(
        r#"{"prompt_tokens":0,"completion_tokens":2,"completion_details":{"reasoning_tokens":5}}"#,
    );
    let (_, completion) = cost(
        &model,
        &usage,
        &PromptExtras::default(),
        moment("2024-06-16T12:00:00Z"),
    );
    assert_eq!(completion, decimal("15"));
}
