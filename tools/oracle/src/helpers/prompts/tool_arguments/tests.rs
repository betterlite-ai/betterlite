#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;
use rstest::rstest;
use serde_json::Value;

#[rstest]
#[case::valid_arguments_parse_directly(
    r#"{"mode":"parse_tool_call_arguments","arguments":"{\"a\":1}"}"#,
    serde_json::json!({"a": 1})
)]
#[case::truncated_arguments_are_repaired(
    r#"{"mode":"parse_tool_call_arguments","arguments":"{\"a\":1,\"b\":[1,2"}"#,
    serde_json::json!({"a": 1, "b": [1, 2]})
)]
#[case::split_concatenated_objects(
    r#"{"mode":"split_concatenated_json_objects","raw":"{\"a\":1}{\"b\":2}"}"#,
    serde_json::json!([{"a": 1}, {"b": 2}])
)]
#[case::salvage_collapses_identical_repeats(
    r#"{"mode":"salvage_concatenated_tool_arguments","raw":"{\"a\":1}{\"a\":1}"}"#,
    serde_json::json!([{"a": 1}])
)]
#[case::tool_call_names_skips_blank_names(
    r#"{"mode":"get_tool_call_names","tools":[{"type":"function","function":{"name":"search"}},{"type":"function","function":{"name":""}}]}"#,
    serde_json::json!(["search"])
)]
#[case::is_function_call_checks_truthiness(
    r#"{"mode":"is_function_call","optional_params":{"functions":{}}}"#,
    serde_json::json!(false)
)]
#[case::check_is_function_call_without_params(
    r#"{"mode":"check_is_function_call"}"#,
    serde_json::json!(false)
)]
#[case::chat_shape_wraps_a_flat_grammar(
    r#"{"mode":"convert_custom_tool_format_to_chat_shape","format_obj":{"type":"grammar","definition":"g"}}"#,
    serde_json::json!({"type": "grammar", "grammar": {"definition": "g"}})
)]
#[case::responses_shape_unwraps_a_wrapped_grammar(
    r#"{"mode":"convert_custom_tool_format_to_responses_shape","format_obj":{"type":"grammar","grammar":{"definition":"g"}}}"#,
    serde_json::json!({"type": "grammar", "definition": "g"})
)]
#[case::cache_breakpoint_inserts_the_marker(
    r#"{"mode":"with_prompt_cache_breakpoint","target":{"role":"user"},"marker":"checkpoint-1"}"#,
    serde_json::json!({"role": "user", "prompt_cache_breakpoint": "checkpoint-1"})
)]
fn tool_arguments_behavior_tests_behavior(#[case] wire: &str, #[case] expected: Value) {
    let input: Input = serde_json::from_str(wire).unwrap();
    assert_eq!(Op::run(&input).unwrap(), expected);
}

#[test]
fn unrepairable_arguments_raise_a_value_error() {
    let input: Input = serde_json::from_str(
        r#"{"mode":"parse_tool_call_arguments","arguments":"{not json","tool_name":"my_tool"}"#,
    )
    .unwrap();
    let error = Op::run(&input).unwrap_err();
    assert_eq!(error.class, "ValueError");
}
