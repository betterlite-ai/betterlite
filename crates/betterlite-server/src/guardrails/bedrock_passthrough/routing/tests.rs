#![cfg(test)]

use rstest::rstest;

use super::{
    EVENT_STREAM_MEDIA_TYPE, Routed, event_stream_endpoint_is_de_anonymizable,
    is_converse_endpoint, is_event_stream_content_type,
};

#[rstest]
#[case::converse("converse")]
#[case::converse_stream("converse-stream")]
#[case::model_path("model/anthropic.claude-v2/converse")]
#[case::trailing_slash("model/anthropic.claude-v2/converse/")]
#[case::many_trailing_slashes("model/anthropic.claude-v2/converse-stream///")]
#[case::absolute_url("https://bedrock-runtime.us-east-1.amazonaws.com/model/x/converse")]
fn converse_endpoints_match(#[case] endpoint: &str) {
    assert!(is_converse_endpoint(endpoint));
    assert!(event_stream_endpoint_is_de_anonymizable(endpoint));
}

#[rstest]
#[case::empty("")]
#[case::bare_slash("/")]
#[case::invoke("model/x/invoke")]
#[case::invoke_stream("model/x/invoke-with-response-stream")]
#[case::prefix("converse-model")]
#[case::suffix("model/conversex")]
#[case::no_separator("conversestream")]
#[case::uppercase("model/x/CONVERSE")]
fn other_endpoints_do_not_match(#[case] endpoint: &str) {
    assert!(!is_converse_endpoint(endpoint));
    assert!(!event_stream_endpoint_is_de_anonymizable(endpoint));
}

#[rstest]
#[case::exact("application/vnd.amazon.eventstream")]
#[case::with_charset("application/vnd.amazon.eventstream; charset=utf-8")]
#[case::fragment_alone("vnd.amazon.eventstream")]
#[case::surrounded("x-application/vnd.amazon.eventstream-x")]
#[case::suffix_after_the_fragment("application/vnd.amazon.eventstream2")]
fn event_stream_content_types_match(#[case] content_type: &str) {
    assert!(is_event_stream_content_type(content_type));
}

#[rstest]
#[case::json("application/json")]
#[case::empty("")]
#[case::dashes("application-vnd.amazon.event-stream")]
fn other_content_types_do_not_match(#[case] content_type: &str) {
    assert!(!is_event_stream_content_type(content_type));
}

#[test]
fn the_media_type_is_amazons() {
    assert_eq!(
        EVENT_STREAM_MEDIA_TYPE,
        "application/vnd.amazon.eventstream"
    );
}

#[test]
fn routed_separates_the_two_owners() {
    assert_ne!(Routed::Handled, Routed::Generic);
}
