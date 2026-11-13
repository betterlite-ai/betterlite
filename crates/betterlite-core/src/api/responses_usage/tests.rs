#![cfg(test)]

use super::{
    ResponsesUsageError, Usage, UsageInput, is_response_api_usage, normalize_usage,
    transform_response_api_usage,
};
use betterlite_models::modality_cost::error::ModalityCostError;
use rstest::rstest;
use serde_json::{Map, Value};
use std::error::Error;

#[test]
fn null_usage_becomes_zero_usage() {
    let expected = value(r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0}"#);
    assert_eq!(
        transform_value(None).expect("normalizes absent usage"),
        expected
    );
    assert_eq!(
        transform_value(Some(&Value::Null)).expect("normalizes null usage"),
        expected
    );
}

#[test]
fn usage_classifier_requires_both_responses_counts() {
    let missing_output: Map<String, Value> =
        serde_json::from_str(r#"{"input_tokens":2}"#).expect("object");
    let both_counts: Map<String, Value> =
        serde_json::from_str(r#"{"input_tokens":2,"output_tokens":3}"#).expect("object");
    assert!(!is_response_api_usage(&missing_output));
    assert!(is_response_api_usage(&both_counts));
}

#[test]
fn existing_chat_usage_keeps_its_inconsistent_total_and_extra() {
    let input = value(r#"{"prompt_tokens":2,"completion_tokens":3,"total_tokens":99,"custom":7}"#);
    assert_eq!(
        transform_value(Some(&input)).expect("ordinary Usage"),
        input
    );
}

#[test]
fn ordinary_usage_mapping_wraps_prompt_details() {
    let input = value(
        r#"{"prompt_tokens":2,"completion_tokens":3,"total_tokens":99,"custom":7,"prompt_tokens_details":{"cached_tokens":"1.0"}}"#,
    );
    let result = transform_value(Some(&input)).expect("ordinary Usage with details");
    assert_eq!(result["total_tokens"], 99_i64);
    assert_eq!(result["custom"], 7_i64);
    assert_eq!(result["prompt_tokens_details"]["cached_tokens"], 1_i64);
    assert_eq!(
        result["prompt_tokens_details"]["cached_tokens_details"],
        Value::Null
    );
    assert!(result.get("completion_tokens_details").is_none());
}

#[test]
fn mapping_with_only_input_tokens_uses_ordinary_usage_construction() {
    let input = value(r#"{"input_tokens":2}"#);
    let result = transform_value(Some(&input)).expect("ordinary Usage mapping");
    assert_eq!(result["prompt_tokens"], 0_i64);
    assert_eq!(result["completion_tokens"], 0_i64);
    assert_eq!(result["total_tokens"], 0_i64);
    assert_eq!(result["input_tokens"], 2_i64);
}

#[test]
fn responses_usage_recomputes_total_and_uses_realtime_detail_aliases() {
    let input = value(
        r#"{"input_tokens":2,"output_tokens":3,"total_tokens":99,"input_token_details":{"audio_tokens":2},"output_token_details":{"audio_tokens":3}}"#,
    );
    let before = input.clone();
    let result = transform_value(Some(&input)).expect("Responses usage");
    assert_eq!(result["prompt_tokens"], 2_i64);
    assert_eq!(result["completion_tokens"], 3_i64);
    assert_eq!(result["total_tokens"], 5_i64);
    assert_eq!(result["prompt_tokens_details"]["audio_tokens"], 2_i64);
    assert_eq!(
        result["prompt_tokens_details"]["cached_tokens_details"],
        Value::Null
    );
    assert_eq!(result["completion_tokens_details"]["audio_tokens"], 3_i64);
    assert!(result.get("input_token_details").is_none());
    assert_eq!(input, before);
}

#[test]
fn typed_response_usage_does_not_fill_realtime_aliases() {
    let input = value(
        r#"{"input_tokens":2,"output_tokens":3,"total_tokens":5,"input_token_details":{"audio_tokens":2},"output_token_details":{"audio_tokens":3}}"#,
    );
    let response = serde_json::from_value(input).expect("typed ResponseApiUsage");
    let projected = super::normalize_and_project(UsageInput::Response(&response))
        .expect("typed usage projection");
    assert!(projected.get("prompt_tokens_details").is_none());
    assert!(projected.get("completion_tokens_details").is_none());
    assert!(projected.get("input_token_details").is_none());
    assert!(projected.get("output_token_details").is_none());
}

#[test]
fn provider_extensions_and_decimal_cost_survive() {
    let input = value(
        r#"{"input_tokens":2,"output_tokens":3,"total_tokens":5,"prompt_tokens":700,"completion_tokens":800,"custom":7,"server_side_tool_usage_details":{"web_search":2},"cost":0.125}"#,
    );
    let result = transform_value(Some(&input)).expect("Responses usage");
    assert_eq!(result["custom"], 7_i64);
    assert_eq!(
        result["server_side_tool_usage_details"]["web_search"],
        2_i64
    );
    assert_eq!(result["cost"], value("0.125"));
    assert_eq!(result["prompt_tokens"], 2_i64);
    assert_eq!(result["completion_tokens"], 3_i64);
    let usage = transform_response_api_usage(Some(&input)).expect("typed conversion");
    assert_eq!(
        normalize_usage(UsageInput::Existing(&usage)).expect("reused Usage"),
        usage
    );
    let no_cost = value(r#"{"input_tokens":2,"output_tokens":3,"total_tokens":5,"cost":null}"#);
    assert!(
        transform_value(Some(&no_cost))
            .expect("null cost")
            .get("cost")
            .is_none()
    );
}

#[test]
fn nested_reasoning_is_removed_only_when_text_includes_it() {
    let input = value(
        r#"{"input_tokens":2,"output_tokens":10,"total_tokens":12,"input_tokens_details":{"cached_tokens":2,"audio_tokens":3,"text_tokens":4,"image_tokens":5,"video_tokens":6,"cache_write_tokens":7,"web_search_requests":8,"google_maps_grounding_requests":9,"cached_tokens_details":{"text_tokens":1}},"output_tokens_details":{"text_tokens":10,"reasoning_tokens":4,"audio_tokens":0,"image_tokens":0}}"#,
    );
    let result = transform_value(Some(&input)).expect("Responses usage");
    assert_eq!(result["prompt_tokens_details"]["cached_tokens"], 2_i64);
    assert_eq!(
        result["prompt_tokens_details"]["cached_tokens_details"]["text_tokens"],
        1_i64
    );
    assert_eq!(result["completion_tokens_details"]["text_tokens"], 6_i64);
    assert_eq!(
        result["completion_tokens_details"]["reasoning_tokens"],
        4_i64
    );
    assert_eq!(
        result["prompt_tokens_details"]["cache_creation_tokens"],
        7_i64
    );
}

#[test]
fn typed_prompt_details_ignore_unpassed_extra_fields() {
    let input = value(
        r#"{"input_tokens":2,"output_tokens":3,"input_tokens_details":{"cached_tokens":1,"cache_creation_tokens":7,"tool_use_tokens":"x","character_count":[1]}}"#,
    );
    let result = transform_value(Some(&input)).expect("typed wrapper ignores unpassed extras");
    let details = &result["prompt_tokens_details"];
    assert!(details.get("cache_write_tokens").is_none());
    assert!(details.get("cache_creation_tokens").is_none());
    assert!(details.get("tool_use_tokens").is_none());
}

#[test]
fn ordinary_prompt_details_use_raw_integer_cache_creation_input_fallback() {
    let input = value(
        r#"{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5,"prompt_tokens_details":{"cache_creation_input_tokens":4}}"#,
    );
    let result = transform_value(Some(&input)).expect("ordinary usage detail fallback");
    assert_eq!(result["prompt_tokens_details"]["cache_write_tokens"], 4_i64);
    assert_eq!(
        result["prompt_tokens_details"]["cache_creation_tokens"],
        4_i64
    );
}

#[test]
fn prompt_detail_integer_strings_integral_numbers_and_bools_are_coerced() {
    let input = value(
        r#"{"input_tokens":2,"output_tokens":3,"input_tokens_details":{"cache_write_tokens":2.0,"web_search_requests":"3","google_maps_grounding_requests":true}}"#,
    );
    let result = transform_value(Some(&input)).expect("lax integer detail values");
    let details = &result["prompt_tokens_details"];
    assert_eq!(details["cache_write_tokens"], 2_i64);
    assert_eq!(details["cache_creation_tokens"], 2_i64);
    assert_eq!(details["web_search_requests"], 3_i64);
    assert_eq!(details["google_maps_grounding_requests"], 1_i64);
}

#[rstest]
#[case::decimal(r#"{"web_search_requests":2.5}"#)]
#[case::mapping(r#"{"web_search_requests":{"count":2}}"#)]
fn prompt_detail_integer_extras_reject_invalid_values(#[case] malformed: &str) {
    let input = value(&format!(
        r#"{{"input_tokens":2,"output_tokens":3,"input_tokens_details":{malformed}}}"#
    ));
    assert!(matches!(
        transform_value(Some(&input)),
        Err(ResponsesUsageError::Json { .. })
    ));
}

#[test]
fn nested_reasoning_overflow_is_reported_with_its_source() {
    let input = value(
        r#"{"input_tokens":0,"output_tokens":0,"output_tokens_details":{"text_tokens":9223372036854775807,"reasoning_tokens":9223372036854775807}}"#,
    );
    let conversion_error =
        transform_response_api_usage(Some(&input)).expect_err("overflow is rejected");
    match conversion_error {
        ResponsesUsageError::ModalityCost { source } => {
            assert!(matches!(
                &source,
                ModalityCostError::ReasoningRemovalOverflow
            ));
            let error = ResponsesUsageError::ModalityCost { source };
            assert!(Error::source(&error).is_some());
        }
        ResponsesUsageError::Json { .. } => panic!("expected modality cost overflow"),
    }
}

#[test]
fn direct_conversion_exposes_typed_representation() {
    let input = value(r#"{"input_tokens":2,"output_tokens":3,"total_tokens":5,"cost":0.125}"#);
    let usage = transform_response_api_usage(Some(&input)).expect("typed conversion");
    assert_eq!(usage.prompt_tokens, 2);
    assert_eq!(usage.cost.expect("cost").to_string(), "0.125");
}

#[test]
fn empty_input_details_keep_zero_cache_and_boolean_output_counts_are_integers() {
    let input = value(
        r#"{"input_tokens":2,"output_tokens":1,"input_tokens_details":{},"output_tokens_details":{"text_tokens":true,"audio_tokens":0,"image_tokens":0}}"#,
    );
    let result = transform_value(Some(&input)).expect("Responses usage");
    assert_eq!(result["prompt_tokens_details"]["cached_tokens"], 0_i64);
    assert_eq!(result["completion_tokens_details"]["text_tokens"], 1_i64);
    assert_eq!(result["completion_tokens_details"]["audio_tokens"], 0_i64);
    assert_eq!(result["completion_tokens_details"]["image_tokens"], 0_i64);
}

#[test]
fn existing_usage_object_is_returned_without_conversion() {
    let usage = Usage {
        prompt_tokens: 2,
        completion_tokens: 3,
        total_tokens: 99,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        cost: None,
        extensions: serde_json::from_str(r#"{"custom":7}"#).expect("extensions"),
    };
    assert_eq!(
        normalize_usage(UsageInput::Existing(&usage)).expect("existing usage"),
        usage
    );
}

#[rstest]
#[case::decimal("2.5")]
#[case::decimal_string(r#""2.5""#)]
#[case::double_digit_separator(r#""1__000""#)]
#[case::leading_digit_separator(r#""_1""#)]
#[case::trailing_digit_separator(r#""1_""#)]
#[case::fractional_digit_separator(r#""2.0_0""#)]
#[case::mapping(r#"{"tokens":2}"#)]
fn output_detail_declared_fields_reject_invalid_integer_values(#[case] malformed: &str) {
    let mut details: Map<String, Value> = Map::new();
    details.insert("text_tokens".to_owned(), value(malformed));
    let mut fields = Map::new();
    fields.insert("input_tokens".to_owned(), Value::from(2_i64));
    fields.insert("output_tokens".to_owned(), Value::from(3_i64));
    fields.insert("output_tokens_details".to_owned(), Value::Object(details));
    assert!(matches!(
        transform_value(Some(&Value::Object(fields))),
        Err(ResponsesUsageError::Json { .. })
    ));
}

#[rstest]
#[case::integer(r#""2""#, 2_i64)]
#[case::decimal(r#""2.0""#, 2_i64)]
#[case::decimal_with_trailing_zeroes(r#""2.00""#, 2_i64)]
#[case::digit_separator(r#""1_000""#, 1000_i64)]
#[case::digit_separator_with_decimal(r#""1_000.0""#, 1000_i64)]
fn output_detail_declared_fields_coerce_integer_strings(
    #[case] reported: &str,
    #[case] expected: i64,
) {
    let input = value(&format!(
        r#"{{"input_tokens":2,"output_tokens":3,"output_tokens_details":{{"text_tokens":{reported}}}}}"#
    ));
    let result = transform_value(Some(&input)).expect("numeric string is a lax integer");
    assert_eq!(result["completion_tokens_details"]["text_tokens"], expected);
}

#[rstest]
#[case::missing(r#"{"input_tokens":2,"output_tokens":3}"#)]
#[case::null(r#"{"input_tokens":2,"output_tokens":3,"total_tokens":null}"#)]
fn missing_or_null_total_is_filled(#[case] source: &str) {
    let input = value(source);
    let result = transform_value(Some(&input)).expect("Responses usage");
    assert_eq!(result["total_tokens"], 5_i64);
}

#[test]
fn plural_input_detail_values_take_precedence_over_realtime_aliases() {
    let input = value(
        r#"{"input_tokens":2,"output_tokens":3,"input_tokens_details":{"audio_tokens":9},"input_token_details":{"audio_tokens":2},"output_tokens_details":{"audio_tokens":8},"output_token_details":{"audio_tokens":3}}"#,
    );
    let result = transform_value(Some(&input)).expect("Responses usage");
    assert_eq!(result["prompt_tokens_details"]["audio_tokens"], 9_i64);
    assert_eq!(result["completion_tokens_details"]["audio_tokens"], 8_i64);
    assert!(result.get("input_token_details").is_none());
    assert!(result.get("output_token_details").is_none());
}

fn value(source: &str) -> Value {
    serde_json::from_str(source).expect("valid test JSON")
}

fn transform_value(usage: Option<&Value>) -> Result<Value, ResponsesUsageError> {
    let input = usage.map_or(UsageInput::Absent, UsageInput::Mapping);
    super::normalize_and_project(input)
}
