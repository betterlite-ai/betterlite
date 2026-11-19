use betterlite_core::schema::SchemaLimits;

use super::support::NoRules;
use super::support::request;
use super::support::value;
use crate::call_setup::correlation::CallOutcome;
use crate::call_wrapper::error::class_name;
use crate::call_wrapper::post_call::{self, PostCallInput, Validation};

const MISMATCHED: &str = r#"{"choices":[{"message":{"content":"{\"answer\":1}"}}]}"#;
const SCHEMA: &str =
    r#"{"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"]}"#;

fn validation_error(enable_json_schema_validation: bool) -> String {
    let outcome = CallOutcome::Response(value(MISMATCHED));
    let request = request(&format!(
        r#"{{"response_format":{{"json_schema":{{"schema":{SCHEMA}}},"type":"json_object","response_schema":{SCHEMA},"enforce_validation":true}}}}"#
    ));
    let input = PostCallInput {
        call_type: "completion",
        model: Some("gpt-4o"),
        request: &request,
        outcome: &outcome,
        validation: Validation {
            enable_json_schema_validation,
            limits: SchemaLimits::default(),
        },
    };
    let error = post_call::run(&NoRules, &input).expect_err("a schema mismatch must fail");
    class_name(&error.class).to_owned()
}

#[test]
fn a_response_format_schema_mismatch_is_a_validation_error() {
    let class = validation_error(true);
    assert_eq!(class, "JSONSchemaValidationError");
}

#[test]
fn an_enforced_response_schema_mismatch_is_a_validation_error() {
    let class = validation_error(false);
    assert_eq!(class, "JSONSchemaValidationError");
}
