#![cfg(test)]

use betterlite_core::routes::CallType;
use rstest::rstest;

use super::route_is;
use crate::straiker::v3::fixtures::{Routes, object};

#[rstest]
#[case::litellm_metadata(
    r#"{"litellm_metadata":{"user_api_key_request_route":"/v1/completions"}}"#,
    CallType::TextCompletion,
    true
)]
#[case::metadata(
    r#"{"metadata":{"user_api_key_request_route":"/v1/messages"}}"#,
    CallType::AnthropicMessages,
    true
)]
#[case::litellm_metadata_overrides_metadata(r#"{"metadata":{"user_api_key_request_route":"/v1/messages"},"litellm_metadata":{"user_api_key_request_route":"/v1/completions"}}"#, CallType::AnthropicMessages, false)]
#[case::other_call_type(
    r#"{"metadata":{"user_api_key_request_route":"/v1/completions"}}"#,
    CallType::AnthropicMessages,
    false
)]
#[case::route_without_call_types(
    r#"{"metadata":{"user_api_key_request_route":"/unknown"}}"#,
    CallType::TextCompletion,
    false
)]
#[case::empty_route(
    r#"{"metadata":{"user_api_key_request_route":""}}"#,
    CallType::TextCompletion,
    false
)]
#[case::route_not_text(
    r#"{"metadata":{"user_api_key_request_route":7}}"#,
    CallType::TextCompletion,
    false
)]
#[case::no_route(r#"{"metadata":{}}"#, CallType::TextCompletion, false)]
#[case::no_metadata("{}", CallType::TextCompletion, false)]
fn detects_route(#[case] source: &str, #[case] call_type: CallType, #[case] expected: bool) {
    assert_eq!(route_is(&object(source), call_type, &Routes), expected);
}
