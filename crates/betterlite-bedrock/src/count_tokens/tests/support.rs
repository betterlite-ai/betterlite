use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use crate::count_tokens::request::{CountTokensInput, CountTokensRequest};
use crate::count_tokens::response::CountTokensResult;

pub(super) fn request(source: &str) -> IndexMap<String, Json> {
    serde_json::from_str(source).unwrap()
}

pub(super) fn value(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn as_json<T>(value: &T) -> Json
where
    T: Serialize,
{
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

pub(super) fn assert_json<T>(actual: &T, expected: &str)
where
    T: Serialize,
{
    assert_eq!(as_json(actual), value(expected));
}

pub(super) fn assert_envelope(actual: &CountTokensRequest, expected: &str) {
    assert_json(actual, expected);
}

pub(super) fn assert_result(actual: &CountTokensResult, expected: &str) {
    assert_json(actual, expected);
}

pub(super) fn with_model(rest: &str) -> IndexMap<String, Json> {
    request(&format!(r#"{{"model": "{MODEL}", {rest}}}"#))
}

pub(super) const MODEL: &str = "anthropic.claude-3-5-sonnet-20241022-v2:0";

pub(super) fn assert_invoke_body(actual: &CountTokensRequest, expected_body: &str) {
    let CountTokensInput::InvokeModel(input) = &actual.input else {
        panic!("expected an invokeModel input");
    };
    assert_eq!(input.body, STANDARD.encode(expected_body));
}
