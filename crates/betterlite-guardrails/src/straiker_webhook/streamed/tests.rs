#![cfg(test)]

use betterlite_prompts::arguments::json::Object;
use rstest::rstest;

use super::request_is_streamed;

#[rstest]
#[case::request_stream(r#"{"stream":true}"#, true)]
#[case::request_is_not_streamed(r#"{"stream":false}"#, false)]
#[case::body_stream(r#"{"proxy_server_request":{"body":{"stream":true}}}"#, true)]
#[case::body_is_not_streamed(r#"{"proxy_server_request":{"body":{"stream":false}}}"#, false)]
#[case::string_stream_is_ignored(r#"{"stream":"true"}"#, false)]
#[case::proxy_request_not_an_object(r#"{"proxy_server_request":"body"}"#, false)]
#[case::no_stream_marker(r#"{"model":"gpt-4o"}"#, false)]
fn straiker_webhook_context_tests_behavior(#[case] source: &str, #[case] expected: bool) {
    let request: Object = serde_json::from_str(source).unwrap();
    assert_eq!(request_is_streamed(&request), expected);
}
