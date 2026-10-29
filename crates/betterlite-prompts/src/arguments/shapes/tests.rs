#![cfg(test)]

use super::{
    check_is_function_call, convert_custom_tool_format_to_chat_shape,
    convert_custom_tool_format_to_responses_shape, get_tool_call_names, is_function_call,
    with_prompt_cache_breakpoint,
};
use crate::arguments::json::{Json, Object};
use rstest::rstest;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("fixture literal is valid JSON")
}

fn object(text: &str) -> Object {
    match json(text) {
        Json::Object(map) => map,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("test fixtures only build objects")
        }
    }
}

enum Operation {
    ToolCallNames(Vec<&'static str>),
    IsFunctionCall(&'static str),
    CheckIsFunctionCall(Option<&'static str>),
    ToChatShape(&'static str),
    ToResponsesShape(&'static str),
    CacheBreakpoint(&'static str, Option<&'static str>),
}

fn run(operation: Operation) -> Json {
    match operation {
        Operation::ToolCallNames(tool_literals) => {
            let tools: Vec<Object> = tool_literals.into_iter().map(object).collect();
            Json::List(
                get_tool_call_names(&tools)
                    .into_iter()
                    .map(Json::Text)
                    .collect(),
            )
        }
        Operation::IsFunctionCall(optional_params) => {
            Json::Bool(is_function_call(&object(optional_params)))
        }
        Operation::CheckIsFunctionCall(optional_params_literal) => {
            let optional_params = optional_params_literal.map(object);
            Json::Bool(check_is_function_call(optional_params.as_ref()))
        }
        Operation::ToChatShape(format_obj) => Json::Object(
            convert_custom_tool_format_to_chat_shape(&object(format_obj)),
        ),
        Operation::ToResponsesShape(format_obj) => Json::Object(
            convert_custom_tool_format_to_responses_shape(&object(format_obj)),
        ),
        Operation::CacheBreakpoint(target, marker_literal) => {
            let marker = marker_literal.map(json);
            Json::Object(with_prompt_cache_breakpoint(
                &object(target),
                marker.as_ref(),
            ))
        }
    }
}

#[rstest]
#[case::tool_call_names_keeps_function_names(
    Operation::ToolCallNames(vec![r#"{"type": "function", "function": {"name": "search"}}"#]),
    json(r#"["search"]"#)
)]
#[case::tool_call_names_skips_non_function_tools(
    Operation::ToolCallNames(vec![r#"{"type": "retrieval"}"#]),
    json("[]")
)]
#[case::tool_call_names_drops_a_blank_name(
    Operation::ToolCallNames(vec![r#"{"type": "function", "function": {"name": ""}}"#]),
    json("[]")
)]
#[case::tool_call_names_skips_a_missing_function_object(
    Operation::ToolCallNames(vec![r#"{"type": "function"}"#]),
    json("[]")
)]
#[case::is_function_call_true_for_a_nonempty_functions_object(
    Operation::IsFunctionCall(r#"{"functions": {"a": 1}}"#),
    json("true")
)]
#[case::is_function_call_false_for_an_empty_functions_object(
    Operation::IsFunctionCall(r#"{"functions": {}}"#),
    json("false")
)]
#[case::is_function_call_false_for_a_false_functions_value(
    Operation::IsFunctionCall(r#"{"functions": false}"#),
    json("false")
)]
#[case::is_function_call_false_without_a_functions_key(
    Operation::IsFunctionCall("{}"),
    json("false")
)]
#[case::check_is_function_call_false_without_optional_params(
    Operation::CheckIsFunctionCall(None),
    json("false")
)]
#[case::check_is_function_call_delegates_to_is_function_call(
    Operation::CheckIsFunctionCall(Some(r#"{"functions": {"a": 1}}"#)),
    json("true")
)]
#[case::chat_shape_wraps_a_flat_grammar(
    Operation::ToChatShape(r#"{"type": "grammar", "definition": "g", "syntax": "lark"}"#),
    json(r#"{"type": "grammar", "grammar": {"definition": "g", "syntax": "lark"}}"#)
)]
#[case::chat_shape_leaves_an_already_wrapped_grammar_alone(
    Operation::ToChatShape(r#"{"type": "grammar", "grammar": {"definition": "g"}}"#),
    json(r#"{"type": "grammar", "grammar": {"definition": "g"}}"#)
)]
#[case::chat_shape_passes_a_non_grammar_type_through(
    Operation::ToChatShape(r#"{"type": "text"}"#),
    json(r#"{"type": "text"}"#)
)]
#[case::responses_shape_unwraps_a_wrapped_grammar(
    Operation::ToResponsesShape(
        r#"{"type": "grammar", "grammar": {"definition": "g", "syntax": "lark"}}"#
    ),
    json(r#"{"type": "grammar", "definition": "g", "syntax": "lark"}"#)
)]
#[case::responses_shape_passes_through_without_a_grammar_key(
    Operation::ToResponsesShape(r#"{"type": "grammar"}"#),
    json(r#"{"type": "grammar"}"#)
)]
#[case::responses_shape_passes_through_a_non_object_grammar_value(
    Operation::ToResponsesShape(r#"{"type": "grammar", "grammar": "not an object"}"#),
    json(r#"{"type": "grammar", "grammar": "not an object"}"#)
)]
#[case::responses_shape_passes_through_a_non_grammar_type(
    Operation::ToResponsesShape(r#"{"type": "text", "grammar": {"definition": "g"}}"#),
    json(r#"{"type": "text", "grammar": {"definition": "g"}}"#)
)]
#[case::cache_breakpoint_leaves_the_target_alone_without_a_marker(
    Operation::CacheBreakpoint(r#"{"role": "user"}"#, None),
    json(r#"{"role": "user"}"#)
)]
#[case::cache_breakpoint_inserts_the_marker(
    Operation::CacheBreakpoint(r#"{"role": "user"}"#, Some(r#""checkpoint-1""#)),
    json(r#"{"role": "user", "prompt_cache_breakpoint": "checkpoint-1"}"#)
)]
fn tool_arguments_behavior_tests_behavior(#[case] operation: Operation, #[case] expected: Json) {
    assert_eq!(run(operation), expected);
}
