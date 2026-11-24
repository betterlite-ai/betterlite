use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use crate::openapi_tools::input_schema::build_input_schema;

use super::support::{json, operation};

fn schema(operation_json: &str) -> Json {
    Json::Object(build_input_schema(&operation(operation_json)))
}

#[test]
fn an_operation_without_parameters_gets_an_empty_object_schema() {
    assert_eq!(
        schema(r#"{"operationId":"list"}"#),
        json(r#"{"type":"object","properties":{},"required":[]}"#)
    );
}

#[rstest]
#[case::without_schema(
    r#"{"parameters":[{"name":"q"}]}"#,
    r#"{"type":"object","properties":{"q":{"type":"string","description":""}},"required":[]}"#
)]
#[case::string_schema(
    r#"{"parameters":[{"name":"q","schema":{"type":"string"}}]}"#,
    r#"{"type":"object","properties":{"q":{"type":"string","description":""}},"required":[]}"#
)]
#[case::integer_schema_with_description(
    r#"{"parameters":[{"name":"limit","schema":{"type":"integer"},"description":"How many"}]}"#,
    r#"{"type":"object","properties":{"limit":{"type":"integer","description":"How many"}},"required":[]}"#
)]
#[case::required_parameter(
    r#"{"parameters":[{"name":"id","required":true}]}"#,
    r#"{"type":"object","properties":{"id":{"type":"string","description":""}},"required":["id"]}"#
)]
#[case::explicitly_optional_parameter(
    r#"{"parameters":[{"name":"id","required":false}]}"#,
    r#"{"type":"object","properties":{"id":{"type":"string","description":""}},"required":[]}"#
)]
#[case::header_and_cookie_parameters_are_included(
    r#"{"parameters":[{"name":"X-Token","in":"header"},{"name":"session","in":"cookie"}]}"#,
    r#"{"type":"object","properties":{"X-Token":{"type":"string","description":""},"session":{"type":"string","description":""}},"required":[]}"#
)]
#[case::nameless_parameters_are_skipped(
    r#"{"parameters":[{"in":"query"},{"name":"kept"}]}"#,
    r#"{"type":"object","properties":{"kept":{"type":"string","description":""}},"required":[]}"#
)]
#[case::parameters_keep_their_order(
    r#"{"parameters":[{"name":"b"},{"name":"a"}]}"#,
    r#"{"type":"object","properties":{"b":{"type":"string","description":""},"a":{"type":"string","description":""}},"required":[]}"#
)]
#[case::required_names_follow_the_parameter_order(
    r#"{"parameters":[{"name":"b","required":true},{"name":"a","required":true}]}"#,
    r#"{"type":"object","properties":{"b":{"type":"string","description":""},"a":{"type":"string","description":""}},"required":["b","a"]}"#
)]
fn turns_parameters_into_properties(#[case] operation_json: &str, #[case] expected: &str) {
    assert_eq!(schema(operation_json), json(expected));
}

#[rstest]
#[case::json_body_with_schema(
    r#"{"requestBody":{"description":"The pet","required":true,"content":{"application/json":{"schema":{"type":"object","properties":{"name":{"type":"string"}}}}}}}"#,
    r#"{"type":"object","properties":{"body":{"type":"object","description":"The pet","properties":{"name":{"type":"string"}}}},"required":["body"]}"#
)]
#[case::json_body_without_description(
    r#"{"requestBody":{"content":{"application/json":{"schema":{"properties":{"name":{"type":"string"}}}}}}}"#,
    r#"{"type":"object","properties":{"body":{"type":"object","description":"Request body","properties":{"name":{"type":"string"}}}},"required":[]}"#
)]
#[case::json_body_without_schema(
    r#"{"requestBody":{"content":{"application/json":{}}}}"#,
    r#"{"type":"object","properties":{"body":{"type":"object","description":"Request body","properties":{}}},"required":[]}"#
)]
#[case::json_body_with_an_empty_description(
    r#"{"requestBody":{"description":"","content":{"application/json":{}}}}"#,
    r#"{"type":"object","properties":{"body":{"type":"object","description":"","properties":{}}},"required":[]}"#
)]
#[case::optional_json_body(
    r#"{"requestBody":{"required":false,"content":{"application/json":{}}}}"#,
    r#"{"type":"object","properties":{"body":{"type":"object","description":"Request body","properties":{}}},"required":[]}"#
)]
#[case::only_body_schema_properties_are_copied(
    r#"{"requestBody":{"content":{"application/json":{"schema":{"required":["a"],"properties":{"a":{"type":"array","items":{"type":"string"}}}}}}}}"#,
    r#"{"type":"object","properties":{"body":{"type":"object","description":"Request body","properties":{"a":{"type":"array","items":{"type":"string"}}}}},"required":[]}"#
)]
fn adds_json_request_bodies_as_body(#[case] operation_json: &str, #[case] expected: &str) {
    assert_eq!(schema(operation_json), json(expected));
}

#[rstest]
#[case::text_plain(r#"{"requestBody":{"content":{"text/plain":{"schema":{"type":"string"}}}}}"#)]
#[case::json_with_charset(r#"{"requestBody":{"content":{"application/json; charset=utf-8":{}}}}"#)]
#[case::wildcard(r#"{"requestBody":{"content":{"*/*":{}}}}"#)]
#[case::empty_content(r#"{"requestBody":{"content":{}}}"#)]
#[case::request_body_without_content(r#"{"requestBody":{"required":true}}"#)]
fn adds_no_body_for_other_content(#[case] operation_json: &str) {
    assert_eq!(
        schema(operation_json),
        json(r#"{"type":"object","properties":{},"required":[]}"#)
    );
}

#[test]
fn the_request_body_replaces_a_parameter_named_body() {
    assert_eq!(
        schema(
            r#"{"parameters":[{"name":"body","description":"parameter"}],"requestBody":{"description":"request","content":{"application/json":{}}}}"#,
        ),
        json(
            r#"{"type":"object","properties":{"body":{"type":"object","description":"request","properties":{}}},"required":[]}"#,
        )
    );
}

#[test]
fn a_required_body_follows_the_required_parameters() {
    assert_eq!(
        schema(
            r#"{"parameters":[{"name":"id","required":true}],"requestBody":{"required":true,"content":{"application/json":{}}}}"#,
        ),
        json(
            r#"{"type":"object","properties":{"id":{"type":"string","description":""},"body":{"type":"object","description":"Request body","properties":{}}},"required":["id","body"]}"#,
        )
    );
}
