#![cfg(test)]

use betterlite_prompts::arguments::json::Json;
use rstest::rstest;
use serde::Serialize;

use super::{response_finish_reason, response_usage};

enum Scenario {
    Reason(&'static str),
    Usage(&'static str),
}

fn parsed(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn as_json<T>(value: &T) -> Json
where
    T: Serialize,
{
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

fn project(scenario: &Scenario) -> Json {
    match scenario {
        Scenario::Reason(source) => as_json(&response_finish_reason(&parsed(source))),
        Scenario::Usage(source) => as_json(&response_usage(&parsed(source))),
    }
}

#[rstest]
#[case::reason_from_top_level_finish_reason(
    Scenario::Reason(r#"{"finish_reason":"stop"}"#),
    parsed(r#""stop""#)
)]
#[case::reason_from_top_level_stop_reason(
    Scenario::Reason(r#"{"stop_reason":"end_turn"}"#),
    parsed(r#""end_turn""#)
)]
#[case::reason_prefers_finish_reason(
    Scenario::Reason(r#"{"stop_reason":"end_turn","finish_reason":"stop"}"#),
    parsed(r#""stop""#)
)]
#[case::reason_empty_top_level_falls_to_choices(
    Scenario::Reason(r#"{"finish_reason":"","choices":[{"finish_reason":"length"}]}"#),
    parsed(r#""length""#)
)]
#[case::reason_first_choice_wins(
    Scenario::Reason(
        r#"{"choices":[{"finish_reason":null,"stop_reason":"max_tokens"},{"finish_reason":"tool_calls"}]}"#
    ),
    parsed(r#""max_tokens""#)
)]
#[case::reason_choice_without_reasons_is_skipped(
    Scenario::Reason(r#"{"choices":[{}, "text", {"stop_reason":"end_turn"}]}"#),
    parsed(r#""end_turn""#)
)]
#[case::reason_choices_not_a_list(
    Scenario::Reason(r#"{"choices":{"finish_reason":"stop"}}"#),
    parsed("null")
)]
#[case::reason_response_not_an_object(Scenario::Reason("[1]"), parsed("null"))]
#[case::usage_openai_keys(
    Scenario::Usage(r#"{"usage":{"prompt_tokens":10,"completion_tokens":4}}"#),
    parsed(r#"{"input_tokens":10,"output_tokens":4}"#)
)]
#[case::usage_anthropic_keys(
    Scenario::Usage(r#"{"usage":{"input_tokens":9,"output_tokens":2}}"#),
    parsed(r#"{"input_tokens":9,"output_tokens":2}"#)
)]
#[case::usage_openai_key_wins_even_at_zero(
    Scenario::Usage(r#"{"usage":{"prompt_tokens":0,"input_tokens":9}}"#),
    parsed(r#"{"input_tokens":0}"#)
)]
#[case::usage_booleans_are_not_counts(
    Scenario::Usage(r#"{"usage":{"prompt_tokens":true,"input_tokens":7,"output_tokens":false}}"#),
    parsed(r#"{"input_tokens":7}"#)
)]
#[case::usage_count_outside_i64_falls_back(
    Scenario::Usage(r#"{"usage":{"prompt_tokens":9223372036854775808,"input_tokens":3}}"#),
    parsed(r#"{"input_tokens":3}"#)
)]
#[case::usage_without_counts_is_absent(Scenario::Usage(r#"{"usage":{}}"#), parsed("null"))]
#[case::usage_null_is_absent(Scenario::Usage(r#"{"usage":null}"#), parsed("null"))]
#[case::usage_absent_is_absent(Scenario::Usage("{}"), parsed("null"))]
fn straiker_webhook_context_tests_behavior(#[case] scenario: Scenario, #[case] expected: Json) {
    assert_eq!(project(&scenario), expected);
}
