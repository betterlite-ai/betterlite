use betterlite_models::controls::json_value::JsonValue;
use rstest::rstest;

use super::{build_request, number, params, value};
use crate::api::responses_transform::error::ResponsesTransformError;
use crate::api::responses_transform::request::to_responses_request;
use crate::api::responses_transform::roles::STEP_TYPE_ROLES;

#[test]
fn step_type_roles_map_the_two_step_types() {
    assert_eq!(
        STEP_TYPE_ROLES,
        [("user_input", "user"), ("model_output", "assistant")]
    );
}

#[test]
fn builds_the_request_from_the_optional_params() {
    let input = value("\"hello\"");
    let built = build_request(
        "gpt-4o",
        Some(&input),
        r#"{
            "system_instruction": "be brief",
            "tools": [{"type": "google_search"}],
            "generation_config": {
                "temperature": 0.2,
                "top_p": 0.9,
                "top_k": 3,
                "max_output_tokens": 128,
                "seed": 7
            },
            "stream": true,
            "store": false,
            "metadata": {"trace": "a"},
            "user": "ada"
        }"#,
        "{}",
    );
    assert_eq!(built.model, "gpt-4o");
    assert_eq!(built.input, Some(JsonValue::Text(String::from("hello"))));
    assert_eq!(built.instructions.as_deref(), Some("be brief"));
    assert_eq!(built.tools, Some(value(r#"[{"type": "google_search"}]"#)));
    assert_eq!(built.temperature, Some(number("0.2")));
    assert_eq!(built.top_p, Some(number("0.9")));
    assert_eq!(built.max_output_tokens, Some(128));
    assert_eq!(built.stream, Some(true));
    assert_eq!(built.store, Some(false));
    assert_eq!(built.metadata, Some(value(r#"{"trace": "a"}"#)));
    assert_eq!(built.user.as_deref(), Some("ada"));
    assert!(built.passthrough.is_empty());
}

#[test]
fn extras_overwrite_mapped_parameters_and_keep_the_rest() {
    let built = build_request(
        "gpt-4o",
        None,
        r#"{"system_instruction": "a", "tools": [{"type": "google_search"}],
            "generation_config": {"temperature": 0.1, "top_p": 0.25, "max_output_tokens": 128},
            "stream": true, "store": true, "user": "ada"}"#,
        r#"{"model": "gpt-4o-mini", "instructions": "b", "system_instruction": "c",
            "tools": [{"type": "code_execution"}], "stream": false, "temperature": 0.9,
            "top_p": 0.5, "max_output_tokens": 256, "store": null, "user": "grace",
            "reasoning": {"effort": "low"}}"#,
    );
    assert_eq!(built.model, "gpt-4o-mini");
    assert_eq!(built.instructions.as_deref(), Some("b"));
    assert_eq!(built.tools, Some(value(r#"[{"type": "code_execution"}]"#)));
    assert_eq!(built.stream, Some(false));
    assert_eq!(built.temperature, Some(number("0.9")));
    assert_eq!(built.top_p, Some(number("0.5")));
    assert_eq!(built.max_output_tokens, Some(256));
    assert_eq!(built.store, None);
    assert_eq!(built.user.as_deref(), Some("grace"));
    assert_eq!(
        built.passthrough,
        params(r#"{"reasoning": {"effort": "low"}, "system_instruction": "c"}"#)
    );
}

#[test]
fn an_explicit_null_in_the_extras_clears_a_mapped_param() {
    let built = build_request(
        "m",
        None,
        r#"{
            "system_instruction": "a",
            "tools": [{"type": "google_search"}],
            "generation_config": {"temperature": 0.1, "top_p": 0.25, "max_output_tokens": 128},
            "stream": true,
            "store": true,
            "user": "ada"
        }"#,
        r#"{
            "instructions": null,
            "tools": null,
            "temperature": null,
            "top_p": null,
            "max_output_tokens": null,
            "stream": null,
            "store": null,
            "user": null
        }"#,
    );
    assert_eq!(built.instructions, None);
    assert_eq!(built.tools, None);
    assert_eq!(built.temperature, None);
    assert_eq!(built.top_p, None);
    assert_eq!(built.max_output_tokens, None);
    assert_eq!(built.stream, None);
    assert_eq!(built.store, None);
    assert_eq!(built.user, None);
    assert!(built.passthrough.is_empty());
}

#[test]
fn null_and_absent_params_stay_unset() {
    let built = build_request(
        "m",
        None,
        r#"{
            "system_instruction": null,
            "tools": null,
            "generation_config": null,
            "stream": null,
            "store": null,
            "metadata": null,
            "user": null
        }"#,
        "{}",
    );
    assert_eq!(built.instructions, None);
    assert_eq!(built.tools, None);
    assert_eq!(built.temperature, None);
    assert_eq!(built.top_p, None);
    assert_eq!(built.max_output_tokens, None);
    assert_eq!(built.stream, None);
    assert_eq!(built.store, None);
    assert_eq!(built.metadata, None);
    assert_eq!(built.user, None);
}

#[test]
fn an_empty_instruction_and_an_empty_tools_list_are_skipped() {
    let built = build_request(
        "m",
        None,
        r#"{"system_instruction": "", "tools": [], "generation_config": {}}"#,
        "{}",
    );
    assert_eq!(built.instructions, None);
    assert_eq!(built.tools, None);
    assert_eq!(built.temperature, None);
}

#[test]
fn an_absent_input_stays_unset() {
    assert_eq!(build_request("m", None, "{}", "{}").input, None);
    let null = value("null");
    assert_eq!(build_request("m", Some(&null), "{}", "{}").input, None);
}

#[rstest]
#[case::instruction(r#"{"system_instruction": 5}"#, "system_instruction: ")]
#[case::tools(r#"{"tools": "x"}"#, "tools: ")]
#[case::stream(r#"{"stream": "true"}"#, "stream: ")]
#[case::store(r#"{"store": 1}"#, "store: ")]
#[case::user(r#"{"user": 5}"#, "user: ")]
#[case::generation_config(r#"{"generation_config": []}"#, "generation_config: ")]
#[case::temperature(
    r#"{"generation_config": {"temperature": "hot"}}"#,
    "generation_config.temperature: "
)]
#[case::top_p(
    r#"{"generation_config": {"top_p": true}}"#,
    "generation_config.top_p: "
)]
#[case::max_output_tokens(
    r#"{"generation_config": {"max_output_tokens": "many"}}"#,
    "generation_config.max_output_tokens: "
)]
#[case::fractional_max_output_tokens(
    r#"{"generation_config": {"max_output_tokens": 1.5}}"#,
    "generation_config.max_output_tokens: "
)]
fn wrong_typed_optional_params_error(#[case] optional_params: &str, #[case] field: &str) {
    let error = to_responses_request("m", None, &params(optional_params), &params("{}"))
        .expect_err("a wrong type must fail");
    assert!(
        error.to_string().starts_with(field),
        "{error} does not name {field}"
    );
}

#[rstest]
#[case::model(r#"{"model": null}"#, "model: expected a string")]
#[case::model_type(r#"{"model": 5}"#, "model: ")]
#[case::instructions(r#"{"instructions": 5}"#, "instructions: ")]
#[case::tools(r#"{"tools": 4}"#, "tools: ")]
#[case::temperature(r#"{"temperature": "hot"}"#, "temperature: ")]
#[case::top_p(r#"{"top_p": []}"#, "top_p: ")]
#[case::max_output_tokens(r#"{"max_output_tokens": "many"}"#, "max_output_tokens: ")]
#[case::fractional_max_output_tokens(r#"{"max_output_tokens": 1.5}"#, "max_output_tokens: ")]
#[case::stream(r#"{"stream": "yes"}"#, "stream: ")]
#[case::store(r#"{"store": 0}"#, "store: ")]
#[case::user(r#"{"user": {}}"#, "user: ")]
fn wrong_typed_extras_error(#[case] extras: &str, #[case] field: &str) {
    let error = to_responses_request("m", None, &params("{}"), &params(extras))
        .expect_err("a wrong type must fail");
    assert!(
        error.to_string().starts_with(field),
        "{error} does not name {field}"
    );
}

#[test]
fn extras_carry_an_input_and_a_metadata_override() {
    let built = build_request(
        "m",
        None,
        r#"{"metadata": {"a": 1}}"#,
        r#"{"input": "override", "metadata": {"b": 2}}"#,
    );
    assert_eq!(built.input, Some(JsonValue::Text(String::from("override"))));
    assert_eq!(built.metadata, Some(value(r#"{"b": 2}"#)));
}

#[test]
fn a_null_input_and_metadata_clear_them() {
    let input = value("\"base\"");
    let built = build_request(
        "m",
        Some(&input),
        r#"{"metadata": {"a": 1}}"#,
        r#"{"input": null, "metadata": null}"#,
    );
    assert_eq!(built.input, None);
    assert_eq!(built.metadata, None);
}

#[test]
fn a_number_input_is_refused() {
    let error = to_responses_request("m", Some(&value("5")), &params("{}"), &params("{}"))
        .expect_err("a number is not an input");
    assert!(matches!(
        error,
        ResponsesTransformError::NotInput { field: "input" }
    ));
}
