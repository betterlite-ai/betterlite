use betterlite_models::decisions::Json;

use super::support::value;
use crate::python_json;
use crate::spend_bodies::placeholder::{placeholder_request_body, placeholder_request_metadata};

fn stored(params: &Json, model_group: &str, raw_model: &str) -> String {
    python_json::dumps(&placeholder_request_body(params, model_group, raw_model))
}

#[test]
fn placeholders_an_existing_model_and_existing_metadata_fields() {
    let params = value(
        r#"{"proxy_server_request": {"body": {"model": "unsafe-name", "metadata": {"model_group": "g", "error_information": {"error_message": "model gpt was bad", "traceback": "gpt failed", "error_class": "x"}, "other": 1}}}}"#,
    );
    assert_eq!(
        stored(&params, "group", "gpt"),
        r#"{"proxy_server_request": {"body": {"model": "unknown-model", "metadata": {"model_group": "group", "error_information": {"error_message": "model unknown-model was bad", "traceback": "unknown-model failed", "error_class": "x"}, "other": 1}}}}"#
    );
}

#[test]
fn does_not_insert_a_missing_model_or_model_group() {
    let params = value(r#"{"proxy_server_request": {"body": {"metadata": {"other": 1}}}}"#);
    assert_eq!(
        stored(&params, "group", "gpt"),
        r#"{"proxy_server_request": {"body": {"metadata": {"other": 1}}}}"#
    );
}

#[test]
fn keeps_a_non_mapping_error_information() {
    let params = value(
        r#"{"proxy_server_request": {"body": {"model": "m", "metadata": {"model_group": "g", "error_information": "plain text"}}}}"#,
    );
    assert_eq!(
        stored(&params, "group", "gpt"),
        r#"{"proxy_server_request": {"body": {"model": "unknown-model", "metadata": {"model_group": "group", "error_information": "plain text"}}}}"#
    );
}

#[rstest::rstest]
#[case::missing_proxy(r#"{"model": "m"}"#)]
#[case::null_proxy(r#"{"proxy_server_request": null}"#)]
#[case::missing_body(r#"{"proxy_server_request": {"headers": {}}}"#)]
fn leaves_params_without_a_stored_body_alone(#[case] source: &str) {
    let params = value(source);
    assert_eq!(stored(&params, "group", "gpt"), python_json::dumps(&params));
}

#[test]
fn metadata_helper_alone_replaces_only_existing_keys() {
    let body = value(r#"{"metadata": {"model_group": "old", "other": 2}}"#);
    assert_eq!(
        python_json::dumps(&placeholder_request_metadata(&body, "new", "raw")),
        r#"{"metadata": {"model_group": "new", "other": 2}}"#
    );
}
