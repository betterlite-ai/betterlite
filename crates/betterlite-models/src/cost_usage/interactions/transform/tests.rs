#![cfg(test)]

use super::transform_interactions_usage;
use crate::cost_usage::error::CostUsageError;
use crate::cost_usage::interactions::record::InteractionsUsage;

fn parse(source: &str) -> InteractionsUsage {
    serde_json::from_str(source).expect("valid JSON object")
}

#[test]
fn document_modality_aggregates_into_text_tokens() {
    let usage = parse(
        r#"{"total_input_tokens":0,"total_output_tokens":0,
        "input_tokens_by_modality":[{"modality":"document","tokens":6}]}"#,
    );
    let cost = transform_interactions_usage(&usage).expect("valid usage");
    let details = cost.prompt_details.expect("modality sums are present");
    assert_eq!(details.text_tokens, Some(6));
}

#[test]
fn tool_use_modality_joins_input_modality_aggregation() {
    let usage = parse(
        r#"{"total_input_tokens":0,"total_output_tokens":0,
        "input_tokens_by_modality":[{"modality":"text","tokens":2}],
        "tool_use_tokens_by_modality":[{"modality":"text","tokens":3}]}"#,
    );
    let cost = transform_interactions_usage(&usage).expect("valid usage");
    let details = cost.prompt_details.expect("modality sums are present");
    assert_eq!(details.text_tokens, Some(5));
}

#[test]
fn zero_cached_modality_blocks_total_cache_fallback() {
    let usage = parse(
        r#"{"total_input_tokens":0,"total_output_tokens":0,"total_cached_tokens":9,
        "input_tokens_by_modality":[{"modality":"text","tokens":5}],
        "cached_tokens_by_modality":[{"modality":"audio","tokens":0}]}"#,
    );
    let cost = transform_interactions_usage(&usage).expect("valid usage");
    let details = cost.prompt_details.expect("modality sums are present");
    assert_eq!(details.text_tokens, Some(5));
}

#[test]
fn reasoning_falls_back_to_thought_tokens() {
    let usage =
        parse(r#"{"total_input_tokens":0,"total_output_tokens":0,"total_thought_tokens":4}"#);
    let cost = transform_interactions_usage(&usage).expect("valid usage");
    let details = cost
        .completion_details
        .expect("reasoning tokens are present");
    assert_eq!(details.reasoning_tokens, Some(4));
}

#[test]
fn explicit_reasoning_tokens_take_precedence_over_thought_tokens() {
    let usage = parse(
        r#"{"total_input_tokens":0,"total_output_tokens":0,
        "total_reasoning_tokens":7,"total_thought_tokens":4}"#,
    );
    let cost = transform_interactions_usage(&usage).expect("valid usage");
    let details = cost
        .completion_details
        .expect("reasoning tokens are present");
    assert_eq!(details.reasoning_tokens, Some(7));
}

#[test]
fn explicit_total_tokens_overrides_computed_sum() {
    let usage = parse(r#"{"total_input_tokens":10,"total_output_tokens":5,"total_tokens":1}"#);
    let cost = transform_interactions_usage(&usage).expect("valid usage");
    assert_eq!(cost.prompt_tokens, 10);
    assert_eq!(cost.completion_tokens, 5);
    assert_eq!(cost.total_tokens, 1);
}

#[test]
fn falsy_total_tokens_falls_back_to_computed_sum() {
    let usage = parse(r#"{"total_input_tokens":10,"total_output_tokens":5,"total_tokens":0}"#);
    let cost = transform_interactions_usage(&usage).expect("valid usage");
    assert_eq!(cost.total_tokens, 15);
}

#[test]
fn google_search_counts_only_matching_mapping_entries() {
    let usage = parse(
        r#"{"total_input_tokens":0,"total_output_tokens":0,
        "grounding_tool_count":[{"type":"google_search","count":2},"not an object",
        {"type":"other","count":9}]}"#,
    );
    let cost = transform_interactions_usage(&usage).expect("valid usage");
    let details = cost
        .prompt_details
        .expect("web search requests are present");
    assert_eq!(details.web_search_requests, Some(2));
}

#[test]
fn absent_modality_sums_and_zero_cache_leave_prompt_details_absent() {
    let usage = parse(r#"{"total_input_tokens":3,"total_output_tokens":0}"#);
    let cost = transform_interactions_usage(&usage).expect("valid usage");
    assert!(cost.prompt_details.is_none());
    assert!(cost.cache_read_tokens.is_none());
}

#[test]
fn zero_web_search_requests_is_none_even_with_other_prompt_details() {
    let usage = parse(
        r#"{"total_input_tokens":0,"total_output_tokens":0,
        "input_tokens_by_modality":[{"modality":"document","tokens":4}],
        "tool_use_tokens_by_modality":[{"modality":"text","tokens":6}]}"#,
    );
    let cost = transform_interactions_usage(&usage).expect("valid usage");
    let details = cost.prompt_details.expect("modality sums are present");
    assert_eq!(details.web_search_requests, None);
}

#[test]
fn zero_reasoning_tokens_is_none_even_with_other_completion_details() {
    let usage = parse(
        r#"{"total_input_tokens":0,"total_output_tokens":0,
        "output_tokens_by_modality":[{"modality":"text","tokens":4}]}"#,
    );
    let cost = transform_interactions_usage(&usage).expect("valid usage");
    let details = cost.completion_details.expect("output sums are present");
    assert_eq!(details.reasoning_tokens, None);
}

#[test]
fn recognized_zero_modality_sum_keeps_prompt_details_present() {
    let usage = parse(
        r#"{"total_input_tokens":0,"total_output_tokens":0,
        "input_tokens_by_modality":[{"modality":"audio","tokens":0}]}"#,
    );
    let cost = transform_interactions_usage(&usage).expect("valid usage");
    let details = cost
        .prompt_details
        .expect("a recognized zero sum still builds details");
    assert_eq!(details.audio_tokens, Some(0));
    assert_eq!(details.text_tokens, None);
}

#[test]
fn malformed_modality_entry_is_an_error() {
    let usage = parse(
        r#"{"total_input_tokens":0,"total_output_tokens":0,
        "input_tokens_by_modality":["not an object"]}"#,
    );
    let error = transform_interactions_usage(&usage).expect_err("non-mapping modality entry");
    assert_eq!(error, CostUsageError::ModalityEntryNotObject);
}

#[test]
fn prompt_total_overflow_is_reported() {
    let usage = parse(&format!(
        r#"{{"total_input_tokens":{},"total_tool_use_tokens":1,"total_output_tokens":0}}"#,
        i64::MAX
    ));
    let error = transform_interactions_usage(&usage).expect_err("overflowing prompt total");
    assert_eq!(error, CostUsageError::PromptTotalOverflow);
}

#[test]
fn total_tokens_overflow_is_reported_only_when_computing_the_fallback() {
    let usage = parse(&format!(
        r#"{{"total_input_tokens":{},"total_output_tokens":{}}}"#,
        i64::MAX,
        i64::MAX
    ));
    let error = transform_interactions_usage(&usage).expect_err("overflowing total fallback");
    assert_eq!(error, CostUsageError::TotalTokensOverflow);
}

#[test]
fn total_input_counter_overflow_is_a_typed_transform_error() {
    let usage = parse(r#"{"total_input_tokens":9223372036854775808,"total_output_tokens":0}"#);
    let error = transform_interactions_usage(&usage).expect_err("counter overflow");
    assert_eq!(error, CostUsageError::CounterOverflow);
}

#[test]
fn truthy_non_array_modality_is_a_typed_transform_error() {
    let usage = parse(r#"{"total_input_tokens":1,"input_tokens_by_modality":"x"}"#);
    let error = transform_interactions_usage(&usage).expect_err("modality list shape");
    assert_eq!(error, CostUsageError::ModalityListNotArray);
}
