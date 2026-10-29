#![cfg(test)]

use super::{
    check_is_function_call, convert_custom_tool_format_to_chat_shape,
    convert_custom_tool_format_to_responses_shape, get_tool_call_names, is_function_call,
    with_prompt_cache_breakpoint,
};
use rstest::rstest;
use serde_json::{Map, Value, json};

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => unreachable!("test fixtures only build objects"),
    }
}

enum Operation {
    ToolCallNames(Vec<Value>),
    IsFunctionCall(Value),
    CheckIsFunctionCall(Option<Value>),
    ToChatShape(Value),
    ToResponsesShape(Value),
    CacheBreakpoint(Value, Option<Value>),
}

fn run(operation: Operation) -> Value {
    match operation {
        Operation::ToolCallNames(tools) => {
            let tools: Vec<Map<String, Value>> = tools.into_iter().map(object).collect();
            Value::from(get_tool_call_names(&tools))
        }
        Operation::IsFunctionCall(optional_params) => {
            Value::from(is_function_call(&object(optional_params)))
        }
        Operation::CheckIsFunctionCall(optional_params) => {
            let optional_params = optional_params.map(object);
            Value::from(check_is_function_call(optional_params.as_ref()))
        }
        Operation::ToChatShape(format_obj) => {
            Value::Object(convert_custom_tool_format_to_chat_shape(&object(format_obj)))
        }
        Operation::ToResponsesShape(format_obj) => Value::Object(
            convert_custom_tool_format_to_responses_shape(&object(format_obj)),
        ),
        Operation::CacheBreakpoint(target, marker) => {
            Value::Object(with_prompt_cache_breakpoint(&object(target), marker.as_ref()))
        }
    }
}

#[rstest]
#[case::tool_call_names_keeps_function_names(
    Operation::ToolCallNames(vec![json!({"type": "function", "function": {"name": "search"}})]),
    json!(["search"])
)]
#[case::tool_call_names_skips_non_function_tools(
    Operation::ToolCallNames(vec![json!({"type": "retrieval"})]),
    json!([])
)]
#[case::tool_call_names_drops_a_blank_name(
    Operation::ToolCallNames(vec![json!({"type": "function", "function": {"name": ""}})]),
    json!([])
)]
#[case::tool_call_names_skips_a_missing_function_object(
    Operation::ToolCallNames(vec![json!({"type": "function"})]),
    json!([])
)]
#[case::is_function_call_true_for_a_nonempty_functions_object(
    Operation::IsFunctionCall(json!({"functions": {"a": 1}})),
    json!(true)
)]
#[case::is_function_call_false_for_an_empty_functions_object(
    Operation::IsFunctionCall(json!({"functions": {}})),
    json!(false)
)]
#[case::is_function_call_false_for_a_false_functions_value(
    Operation::IsFunctionCall(json!({"functions": false})),
    json!(false)
)]
#[case::is_function_call_false_without_a_functions_key(
    Operation::IsFunctionCall(json!({})),
    json!(false)
)]
#[case::check_is_function_call_false_without_optional_params(
    Operation::CheckIsFunctionCall(None),
    json!(false)
)]
#[case::check_is_function_call_delegates_to_is_function_call(
    Operation::CheckIsFunctionCall(Some(json!({"functions": {"a": 1}}))),
    json!(true)
)]
#[case::chat_shape_wraps_a_flat_grammar(
    Operation::ToChatShape(json!({"type": "grammar", "definition": "g", "syntax": "lark"})),
    json!({"type": "grammar", "grammar": {"definition": "g", "syntax": "lark"}})
)]
#[case::chat_shape_leaves_an_already_wrapped_grammar_alone(
    Operation::ToChatShape(json!({"type": "grammar", "grammar": {"definition": "g"}})),
    json!({"type": "grammar", "grammar": {"definition": "g"}})
)]
#[case::chat_shape_passes_a_non_grammar_type_through(
    Operation::ToChatShape(json!({"type": "text"})),
    json!({"type": "text"})
)]
#[case::responses_shape_unwraps_a_wrapped_grammar(
    Operation::ToResponsesShape(json!({"type": "grammar", "grammar": {"definition": "g", "syntax": "lark"}})),
    json!({"type": "grammar", "definition": "g", "syntax": "lark"})
)]
#[case::responses_shape_passes_through_without_a_grammar_key(
    Operation::ToResponsesShape(json!({"type": "grammar"})),
    json!({"type": "grammar"})
)]
#[case::responses_shape_passes_through_a_non_object_grammar_value(
    Operation::ToResponsesShape(json!({"type": "grammar", "grammar": "not an object"})),
    json!({"type": "grammar", "grammar": "not an object"})
)]
#[case::responses_shape_passes_through_a_non_grammar_type(
    Operation::ToResponsesShape(json!({"type": "text", "grammar": {"definition": "g"}})),
    json!({"type": "text", "grammar": {"definition": "g"}})
)]
#[case::cache_breakpoint_leaves_the_target_alone_without_a_marker(
    Operation::CacheBreakpoint(json!({"role": "user"}), None),
    json!({"role": "user"})
)]
#[case::cache_breakpoint_inserts_the_marker(
    Operation::CacheBreakpoint(json!({"role": "user"}), Some(json!("checkpoint-1"))),
    json!({"role": "user", "prompt_cache_breakpoint": "checkpoint-1"})
)]
fn tool_arguments_behavior_tests_behavior(#[case] operation: Operation, #[case] expected: Value) {
    assert_eq!(run(operation), expected);
}
