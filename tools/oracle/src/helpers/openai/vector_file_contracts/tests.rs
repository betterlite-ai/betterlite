#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;
use serde_json::{Value, json};

fn input(value: Value) -> Input {
    serde_json::from_value(value).unwrap()
}

#[test]
fn unknown_record_kind_is_not_an_input() {
    let decoded: Result<Input, _> = serde_json::from_value(json!({
        "operation": "attachment_record_fields",
        "records": [{"kind": "mystery", "value": {}}],
    }));
    decoded.unwrap_err();
}

#[test]
fn invalid_record_values_are_validation_errors() {
    let error = Op::run(&input(json!({
        "operation": "attachment_record_fields",
        "records": [{"kind": "file_object", "value": {"status": "queued"}}],
    })))
    .unwrap_err();
    assert_eq!(error.class, "ValidationError");
}

#[test]
fn provider_defaults_report_a_missing_api_base_as_a_value_error() {
    let output = Op::run(&input(json!({
        "operation": "file_provider_defaults",
        "optional_params": {"limit": 1_u32},
        "non_default_params": {},
        "headers": {"X": "y"},
        "request_data": {},
        "api_base": "https://api.example.test",
        "api_key": null,
        "api_bases": [null, ""],
        "error": {"message": "bad", "status_code": 429_u32, "headers": {"Retry-After": 3_u32}},
    })))
    .unwrap();
    assert_eq!(
        output["urls"],
        json!([{"error": {"class": "ValueError", "message": "api_base is required"}}, ""])
    );
    assert_eq!(
        output["error"],
        json!({"status_code": 429_u32, "message": "bad", "headers": {"Retry-After": 3_u32}})
    );
    assert_eq!(output["signed_body"], Value::Null);
}

#[test]
fn filters_apply_the_operation_key_set() {
    let output = Op::run(&input(json!({
        "operation": "list_query_filter",
        "queries": [{"after": null, "limit": 0_u32, "unknown": 1_u32}],
    })))
    .unwrap();
    assert_eq!(output, json!([{"limit": 0_u32}]));
}
