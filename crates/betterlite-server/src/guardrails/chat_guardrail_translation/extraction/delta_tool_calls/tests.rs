#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::streamed_delta_tool_calls;
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{guardrail, json};

#[rstest]
#[case::tool_calls(
    json(r#"{"tool_calls": [{"id": "1"}, {"id": "2"}]}"#),
    json(r#"[{"id": "1"}, {"id": "2"}]"#)
)]
#[case::legacy_function_call(
    json(r#"{"function_call": {"name": "f"}}"#),
    json(r#"[{"name": "f"}]"#)
)]
#[case::tool_calls_come_before_the_legacy_call(
    json(r#"{"function_call": {"name": "f"}, "tool_calls": [{"id": "1"}]}"#),
    json(r#"[{"id": "1"}, {"name": "f"}]"#)
)]
#[case::null_function_call(
    json(r#"{"function_call": null, "tool_calls": [{"id": "1"}]}"#),
    json(r#"[{"id": "1"}]"#)
)]
#[case::tool_calls_that_are_not_a_list(
    json(r#"{"tool_calls": {"id": "1"}, "function_call": {"name": "f"}}"#),
    json(r#"[{"name": "f"}]"#)
)]
#[case::null_tool_calls(json(r#"{"tool_calls": null}"#), json("[]"))]
#[case::empty_tool_calls(json(r#"{"tool_calls": []}"#), json("[]"))]
#[case::neither(json(r#"{"content": "hi"}"#), json("[]"))]
#[case::delta_that_is_not_an_object(json(r#""hi""#), json("[]"))]
fn a_delta_lists_its_tool_calls_then_its_function_call(
    #[case] delta: JsonValue,
    #[case] expected: JsonValue,
) {
    assert_eq!(
        serde_json::to_value(streamed_delta_tool_calls(&guardrail(delta))).unwrap(),
        expected
    );
}
