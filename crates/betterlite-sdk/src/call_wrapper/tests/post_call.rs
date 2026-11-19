use betterlite_core::schema::SchemaLimits;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::SchemaValidationDetails;

use super::support::NoRules;
use super::support::Stub;
use super::support::chat_request;
use super::support::fixture;
use super::support::request;
use super::support::response;
use super::support::value;
use crate::call_setup::correlation::CallOutcome;
use crate::call_wrapper::error::CallError;
use crate::call_wrapper::error::class_name;
use crate::call_wrapper::post_call::{self, PostCallInput, Validation};
use crate::call_wrapper::state::WrapperOptions;

const MISMATCHED: &str = r#"{"choices":[{"message":{"content":"{\"answer\":1}"}}]}"#;
const SCHEMA: &str =
    r#"{"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"]}"#;
const MESSAGE: &str = "litellm.JSONSchemaValidationError: model=gpt-4o, returned an invalid response={\"answer\":1}, for schema={}.\nAccess raw response with `e.raw_response`";

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
fn a_schema_validation_error_speaks_the_litellm_message() {
    let error = CallError::from(ErrorClass::JsonSchemaValidation(Box::new(
        SchemaValidationDetails {
            model: String::from("gpt-4o"),
            provider: String::from("openai"),
            raw_response: String::from("{\"answer\":1}"),
            schema: String::from("{}"),
        },
    )));
    assert_eq!(error.to_string(), MESSAGE);
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

#[tokio::test]
async fn async_call_validates_the_response_format() {
    let mut fixture = fixture();
    let stub = Stub::responding("acompletion", response(value(MISMATCHED)));
    let request = chat_request(
        r#"{"enable_json_schema_validation":true,"response_format":{"json_schema":{"schema":{"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"]}}}}"#,
    );
    let error = fixture
        .wrapper
        .call(&stub, request, WrapperOptions::default())
        .await
        .expect_err("the schema mismatch fails the call");
    assert_eq!(class_name(&error.class), "JSONSchemaValidationError");
}
