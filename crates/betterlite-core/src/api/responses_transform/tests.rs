#![cfg(test)]

mod input;
mod request;
mod response;

use betterlite_context::non_default_params::Params;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use serde_json::Number;

use super::request::{ResponsesRequest, to_responses_request};

fn params(source: &str) -> Params {
    serde_json::from_str(source).expect("the fixture should parse")
}

fn value(source: &str) -> JsonValue {
    serde_json::from_str(source).expect("the fixture should parse")
}

fn number(source: &str) -> Number {
    serde_json::from_str(source).expect("the fixture should parse")
}

fn parsed(source: &str) -> ResponsesApiResponse {
    serde_json::from_str(source).expect("the fixture should parse")
}

fn build_request(
    model: &str,
    input: Option<&JsonValue>,
    optional_params: &str,
    extras: &str,
) -> ResponsesRequest {
    to_responses_request(model, input, &params(optional_params), &params(extras))
        .expect("the request should build")
}

fn request_error(input: &str) -> String {
    to_responses_request("m", Some(&value(input)), &params("{}"), &params("{}"))
        .expect_err("the input must be refused")
        .to_string()
}
