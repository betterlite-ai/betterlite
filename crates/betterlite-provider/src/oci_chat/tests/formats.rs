use rstest::rstest;

use crate::oci_chat::tests::support::{body, chat_field, rejected};

const LLAMA: &str = "meta.llama-3.3-70b-instruct";
const COHERE: &str = "cohere.command-r";

const MISSING_NAME: &str =
    "Invalid tool_choice for OCI: 'FUNCTION' type requires a non-empty function name";

#[rstest]
#[case::auto_text(r#""Auto""#, r#"{"type": "AUTO"}"#)]
#[case::none_text(r#""none""#, r#"{"type": "NONE"}"#)]
#[case::any_text(r#""ANY""#, r#"{"type": "REQUIRED"}"#)]
#[case::required_text(r#""required""#, r#"{"type": "REQUIRED"}"#)]
#[case::function_name_text(r#""My_Function""#, r#"{"type": "FUNCTION", "name": "My_Function"}"#)]
#[case::nested_function(
    r#"{"type": "function", "function": {"name": "lookup"}}"#,
    r#"{"type": "FUNCTION", "name": "lookup"}"#
)]
#[case::flat_function(
    r#"{"type": "function", "name": "lookup"}"#,
    r#"{"type": "FUNCTION", "name": "lookup"}"#
)]
#[case::nested_name_wins(
    r#"{"type": "function", "function": {"name": "nested"}, "name": "flat"}"#,
    r#"{"type": "FUNCTION", "name": "nested"}"#
)]
#[case::function_that_is_not_an_object(
    r#"{"type": "function", "function": "nested", "name": "flat"}"#,
    r#"{"type": "FUNCTION", "name": "flat"}"#
)]
#[case::required_object(r#"{"type": "required"}"#, r#"{"type": "REQUIRED"}"#)]
#[case::auto_object(r#"{"type": "AUTO"}"#, r#"{"type": "AUTO"}"#)]
#[case::none_object(r#"{"type": "None"}"#, r#"{"type": "NONE"}"#)]
#[case::null("null", "null")]
#[tokio::test]
async fn tool_choice_strings_and_objects(#[case] given: &str, #[case] expected: &str) {
    let optional = format!(r#"{{"tool_choice": {given}}}"#);
    assert_eq!(
        chat_field(LLAMA, &optional, "toolChoice").await,
        body(expected)
    );
}

#[rstest]
#[case::type_not_text(
    r#"{"type": 5}"#,
    r#"Invalid tool_choice for OCI: missing or non-string 'type' in {"type":5}"#
)]
#[case::type_missing(
    r#"{"name": "lookup"}"#,
    r#"Invalid tool_choice for OCI: missing or non-string 'type' in {"name":"lookup"}"#
)]
#[case::function_without_name(r#"{"type": "function"}"#, MISSING_NAME)]
#[case::function_with_empty_name(r#"{"type": "function", "name": ""}"#, MISSING_NAME)]
#[case::function_with_empty_nested_name(
    r#"{"type": "function", "function": {"name": ""}}"#,
    MISSING_NAME
)]
#[case::function_with_name_that_is_not_text(
    r#"{"type": "function", "function": {"name": 5}}"#,
    MISSING_NAME
)]
#[case::unknown_type(
    r#"{"type": "weird"}"#,
    "Invalid tool_choice for OCI: unsupported type weird; expected one of 'FUNCTION', 'AUTO', 'NONE', 'REQUIRED'"
)]
#[case::list(
    "[1, 2]",
    "Invalid tool_choice for OCI: expected str or dict, got list"
)]
#[case::integer("5", "Invalid tool_choice for OCI: expected str or dict, got int")]
#[case::fraction("1.5", "Invalid tool_choice for OCI: expected str or dict, got float")]
#[case::boolean("true", "Invalid tool_choice for OCI: expected str or dict, got bool")]
#[tokio::test]
async fn tool_choice_rejects_bad_shapes(#[case] given: &str, #[case] message: &str) {
    let optional = format!(r#"{{"tool_choice": {given}}}"#);
    let refusal = rejected(LLAMA, &optional).await;
    assert_eq!(refusal.status, 400);
    assert_eq!(refusal.message, message);
}

#[rstest]
#[case::generic_text(LLAMA, r#"{"type": "text"}"#, r#"{"type": "TEXT"}"#)]
#[case::cohere_text(COHERE, r#"{"type": "Text"}"#, r#"{"type": "TEXT"}"#)]
#[case::generic_json_object(LLAMA, r#"{"type": "json_object"}"#, r#"{"type": "JSON_OBJECT"}"#)]
#[case::generic_upper_case(LLAMA, r#"{"type": "JSON_OBJECT"}"#, r#"{"type": "JSON_OBJECT"}"#)]
#[case::generic_json(LLAMA, r#"{"type": "json"}"#, r#"{"type": "JSON_OBJECT"}"#)]
#[case::cohere_json_object(COHERE, r#"{"type": "json_object"}"#, r#"{"type": "JSON_OBJECT"}"#)]
#[case::generic_schema(
    LLAMA,
    r#"{"type": "json_schema", "json_schema": {"name": "n", "description": "d", "schema": {"type": "object"}, "strict": true}}"#,
    r#"{"type": "JSON_SCHEMA", "jsonSchema": {"name": "n", "description": "d", "schema": {"type": "object"}, "isStrict": true}}"#
)]
#[case::generic_schema_without_a_name(
    LLAMA,
    r#"{"type": "json_schema", "json_schema": {"schema": {"type": "object"}}}"#,
    r#"{"type": "JSON_SCHEMA", "jsonSchema": {"name": "response", "schema": {"type": "object"}}}"#
)]
#[case::generic_schema_with_null_keys(
    LLAMA,
    r#"{"type": "json_schema", "json_schema": {"name": null, "description": null, "schema": null, "strict": null}}"#,
    r#"{"type": "JSON_SCHEMA", "jsonSchema": {"name": "response"}}"#
)]
#[case::generic_schema_keeps_a_false_strict(
    LLAMA,
    r#"{"type": "json_schema", "json_schema": {"name": "n", "strict": false}}"#,
    r#"{"type": "JSON_SCHEMA", "jsonSchema": {"name": "n", "isStrict": false}}"#
)]
#[case::cohere_schema(
    COHERE,
    r#"{"type": "json_schema", "json_schema": {"name": "n", "schema": {"type": "object"}, "strict": true}}"#,
    r#"{"type": "JSON_OBJECT", "schema": {"type": "object"}}"#
)]
#[case::cohere_schema_with_a_null_schema(
    COHERE,
    r#"{"type": "json_schema", "json_schema": {"name": "n", "schema": null}}"#,
    r#"{"type": "JSON_OBJECT"}"#
)]
#[case::cohere_schema_without_a_schema(
    COHERE,
    r#"{"type": "json_schema", "json_schema": {"name": "n"}}"#,
    r#"{"type": "JSON_OBJECT"}"#
)]
#[case::cohere_schema_without_an_object(
    COHERE,
    r#"{"type": "json_schema", "json_schema": "n"}"#,
    r#"{"type": "JSON_OBJECT"}"#
)]
#[tokio::test]
async fn response_format_maps_per_vendor(
    #[case] model: &str,
    #[case] given: &str,
    #[case] expected: &str,
) {
    let optional = format!(r#"{{"response_format": {given}}}"#);
    assert_eq!(
        chat_field(model, &optional, "responseFormat").await,
        body(expected)
    );
}

#[rstest]
#[case::missing_schema(r#"{"type": "json_schema"}"#)]
#[case::schema_not_an_object(r#"{"type": "json_schema", "json_schema": "n"}"#)]
#[case::schema_null(r#"{"type": "json_schema", "json_schema": null}"#)]
#[tokio::test]
async fn response_format_rejects_a_missing_json_schema(#[case] given: &str) {
    let optional = format!(r#"{{"response_format": {given}}}"#);
    let refusal = rejected(LLAMA, &optional).await;
    assert_eq!(refusal.status, 400);
    assert_eq!(
        refusal.message,
        "response_format type 'json_schema' requires a 'json_schema' object"
    );
}

#[tokio::test]
async fn response_format_rejects_a_type_that_is_not_text() {
    let refusal = rejected(LLAMA, r#"{"response_format": {"type": 5}}"#).await;
    assert_eq!(refusal.status, 400);
    assert_eq!(refusal.message, "response_format type must be a string");
}
