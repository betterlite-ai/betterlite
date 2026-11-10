use rstest::rstest;

use super::support::{json, serialized};
use crate::datadog_llm_obs::tool_definitions::to_dd_tool_definitions;

fn definitions(parameters: &str) -> String {
    serialized(&to_dd_tool_definitions(Some(&json(parameters))))
}

#[test]
fn projects_function_wrapped_and_bare_tool_definitions() {
    assert_eq!(
        definitions(
            r#"{"tools": [
                {"type": "function", "function": {"name": "get_weather", "description": "Weather by city", "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}}},
                {"name": "bare_tool", "description": "Bare shape", "input_schema": {"type": "object"}}
            ]}"#
        ),
        concat!(
            r#"[{"name":"get_weather","description":"Weather by city","schema":{"type":"object","properties":{"city":{"type":"string"}}}},"#,
            r#"{"name":"bare_tool","description":"Bare shape","schema":{"type":"object"}}]"#
        )
    );
}

#[test]
fn prefers_the_input_schema_when_parameters_is_empty() {
    assert_eq!(
        definitions(
            r#"{"tools": [{"type": "function", "function": {"name": "t", "parameters": {}, "input_schema": {"type": "object", "properties": {"q": {"type": "string"}}}}}]}"#
        ),
        r#"[{"name":"t","description":"","schema":{"type":"object","properties":{"q":{"type":"string"}}}}]"#
    );
}

#[test]
fn drops_tools_without_a_name() {
    assert_eq!(
        definitions(
            r#"{"tools": [{"type": "function", "function": {"description": "no name", "parameters": {"type": "object"}}}]}"#
        ),
        "[]"
    );
}

#[test]
fn omits_a_schema_that_is_not_an_object() {
    assert_eq!(
        definitions(
            r#"{"tools": [{"type": "function", "function": {"name": "t", "parameters": "not a schema"}}]}"#
        ),
        r#"[{"name":"t","description":""}]"#
    );
}

#[test]
fn falls_back_to_functions_when_tools_is_empty() {
    assert_eq!(
        definitions(
            r#"{"tools": [], "functions": [{"name": "legacy", "description": "Old shape", "parameters": {"type": "object"}}]}"#
        ),
        r#"[{"name":"legacy","description":"Old shape","schema":{"type":"object"}}]"#
    );
}

#[rstest]
#[case::not_an_object("[]")]
#[case::null("null")]
#[case::tools_not_a_list(r#"{"tools": "get_weather"}"#)]
#[case::missing_tools(r#"{"model": "gpt-4"}"#)]
fn a_shape_without_tools_has_no_tool_definitions(#[case] parameters: &str) {
    assert_eq!(definitions(parameters), "[]");
}

#[test]
fn an_absent_model_parameters_has_no_tool_definitions() {
    assert_eq!(serialized(&to_dd_tool_definitions(None)), "[]");
}
