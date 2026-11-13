#![cfg(test)]

use rstest::rstest;

use super::support::object;
use crate::gigachat_chat::options::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use crate::openai_base_config::Object;

fn map(params: &str) -> Object {
    map_openai_params(&object(params), Object::new()).unwrap()
}

#[test]
fn supported_params_are_the_eleven_litellm_names_in_order() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "stream",
            "temperature",
            "top_p",
            "max_tokens",
            "max_completion_tokens",
            "stop",
            "tools",
            "tool_choice",
            "functions",
            "function_call",
            "response_format",
        ]
    );
}

#[rstest]
#[case::stream(r#"{"stream": true}"#, r#"{"stream": true}"#)]
#[case::top_p(r#"{"top_p": 0.5}"#, r#"{"top_p": 0.5}"#)]
#[case::max_tokens(r#"{"max_tokens": 12}"#, r#"{"max_tokens": 12}"#)]
#[case::max_completion_tokens(r#"{"max_completion_tokens": 12}"#, r#"{"max_tokens": 12}"#)]
#[case::both_max_keys(
    r#"{"max_tokens": 100, "max_completion_tokens": 200}"#,
    r#"{"max_tokens": 200}"#
)]
#[case::top_p_wins_over_a_zero_temperature(
    r#"{"temperature": 0, "top_p": 0.5}"#,
    r#"{"top_p": 0.5}"#
)]
#[case::temperature(r#"{"temperature": 0.5}"#, r#"{"temperature": 0.5}"#)]
#[case::stop(r#"{"stop": ["a"]}"#, "{}")]
#[case::unknown(r#"{"seed": 4}"#, "{}")]
#[case::null_parameter(r#"{"temperature": null, "stream": null}"#, "{}")]
fn each_supported_option_maps_to_its_gigachat_name(#[case] params: &str, #[case] expected: &str) {
    assert_eq!(map(params), object(expected));
}

#[rstest]
#[case::integer_zero(r#"{"temperature": 0}"#)]
#[case::float_zero(r#"{"temperature": 0.0}"#)]
fn temperature_zero_becomes_top_p_zero(#[case] params: &str) {
    let mapped = map(params);
    assert_eq!(mapped, object(r#"{"top_p": 0}"#));
    assert!(!mapped.contains_key("temperature"));
}

#[test]
fn tools_become_functions_with_their_three_fields() {
    let mapped = map(
        r#"{"tools": [{"type": "function", "function": {"name": "lookup", "description": "find a city", "parameters": {"type": "object"}}}]}"#,
    );
    assert_eq!(
        mapped,
        object(
            r#"{"functions": [{"name": "lookup", "description": "find a city", "parameters": {"type": "object"}}]}"#
        )
    );
}

#[test]
fn a_function_without_its_optional_fields_gets_empty_defaults() {
    let mapped = map(r#"{"tools": [{"type": "function", "function": {"name": "lookup"}}]}"#);
    assert_eq!(
        mapped,
        object(r#"{"functions": [{"name": "lookup", "description": "", "parameters": {}}]}"#)
    );
}

#[test]
fn tools_that_are_not_function_tools_are_dropped() {
    let mapped = map(
        r#"{"tools": [{"type": "custom", "function": {"name": "gone"}}, "plain", {"type": "function", "function": {"name": "keep"}}]}"#,
    );
    assert_eq!(
        mapped,
        object(r#"{"functions": [{"name": "keep", "description": "", "parameters": {}}]}"#)
    );
}

#[test]
fn an_empty_tools_list_still_sends_functions() {
    assert_eq!(map(r#"{"tools": []}"#), object(r#"{"functions": []}"#));
}

#[test]
fn a_tools_value_that_is_not_a_list_is_rejected() {
    let error = map_openai_params(&object(r#"{"tools": "lookup"}"#), Object::new()).unwrap_err();
    assert_eq!(error.status_code, 400);
}

#[rstest]
#[case::none(r#""none""#, r#"{"function_call": "none"}"#)]
#[case::auto(r#""auto""#, r#"{"function_call": "auto"}"#)]
#[case::required(r#""required""#, r#"{"function_call": "auto"}"#)]
fn tool_choice_modes_become_a_function_call(#[case] choice: &str, #[case] expected: &str) {
    let params = format!(r#"{{"tool_choice": {choice}}}"#);
    assert_eq!(map(&params), object(expected));
}

#[test]
fn a_forced_tool_becomes_a_named_function_call() {
    let mapped = map(r#"{"tool_choice": {"type": "function", "function": {"name": "lookup"}}}"#);
    assert_eq!(mapped, object(r#"{"function_call": {"name": "lookup"}}"#));
}

#[rstest]
#[case::empty_name(r#"{"tool_choice": {"type": "function", "function": {"name": ""}}}"#)]
#[case::unknown_mode(r#"{"tool_choice": "sometimes"}"#)]
#[case::other_type(r#"{"tool_choice": {"type": "other", "function": {"name": "lookup"}}}"#)]
#[case::not_a_choice(r#"{"tool_choice": 5}"#)]
fn a_tool_choice_litellm_cannot_map_sets_nothing(#[case] params: &str) {
    assert_eq!(map(params), object("{}"));
}

#[test]
fn legacy_functions_and_function_call_are_copied_as_given() {
    let mapped = map(r#"{"functions": [{"name": "lookup"}], "function_call": {"name": "lookup"}}"#);
    assert_eq!(
        mapped,
        object(r#"{"functions": [{"name": "lookup"}], "function_call": {"name": "lookup"}}"#)
    );
}

#[test]
fn existing_optional_params_survive_and_a_mapped_one_wins() {
    let mapped = map_openai_params(
        &object(r#"{"top_p": 0.5}"#),
        object(r#"{"top_p": 0.1, "repetition_penalty": 1.1}"#),
    )
    .unwrap();
    assert_eq!(
        mapped,
        object(r#"{"top_p": 0.5, "repetition_penalty": 1.1}"#)
    );
}

#[test]
fn a_json_schema_response_format_becomes_a_function_and_a_named_choice() {
    let mapped = map(
        r#"{"response_format": {"type": "json_schema", "json_schema": {"name": "city", "schema": {"type": "object"}}}}"#,
    );
    assert_eq!(
        mapped,
        object(
            r#"{"functions": [{"name": "city", "description": "Output structured response: city", "parameters": {"type": "object"}}], "function_call": {"name": "city"}, "_structured_output": true}"#
        )
    );
}

#[test]
fn a_json_schema_response_format_appends_to_existing_functions() {
    let mapped = map_openai_params(
        &object(r#"{"response_format": {"type": "json_schema", "json_schema": {"schema": {}}}}"#),
        object(r#"{"functions": [{"name": "lookup"}]}"#),
    )
    .unwrap();
    assert_eq!(
        mapped,
        object(
            r#"{"functions": [{"name": "lookup"}, {"name": "structured_output", "description": "Output structured response: structured_output", "parameters": {}}], "function_call": {"name": "structured_output"}, "_structured_output": true}"#
        )
    );
}

#[test]
fn tools_overwrite_the_functions_a_response_format_added() {
    let mapped = map(
        r#"{"tools": [{"type": "function", "function": {"name": "lookup"}}], "response_format": {"type": "json_schema", "json_schema": {"name": "city", "schema": {}}}}"#,
    );
    assert_eq!(
        mapped,
        object(
            r#"{"functions": [{"name": "lookup", "description": "", "parameters": {}}], "function_call": {"name": "city"}, "_structured_output": true}"#
        )
    );
}

#[test]
fn a_response_format_that_is_not_json_schema_sets_nothing() {
    assert_eq!(
        map(r#"{"response_format": {"type": "json_object"}}"#),
        object("{}")
    );
}

#[rstest]
#[case::non_object_format(r#"{"response_format": "json"}"#)]
#[case::non_object_schema(r#"{"response_format": {"type": "json_schema", "json_schema": 5}}"#)]
#[case::non_string_name(
    r#"{"response_format": {"type": "json_schema", "json_schema": {"name": 5}}}"#
)]
#[case::non_object_parameters(
    r#"{"response_format": {"type": "json_schema", "json_schema": {"schema": "x"}}}"#
)]
fn a_response_format_of_the_wrong_shape_is_rejected(#[case] params: &str) {
    let error = map_openai_params(&object(params), Object::new()).unwrap_err();
    assert_eq!(error.status_code, 400);
}

#[test]
fn a_parse_failure_keeps_its_serde_error_as_the_source() {
    let params = object(r#"{"response_format": {"type": "json_schema", "json_schema": 5}}"#);
    let error = map_openai_params(&params, Object::new()).unwrap_err();
    assert!(error.source.is_some());
}
