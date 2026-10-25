#![cfg(test)]
use super::{
    ResponseHandling, buffer_response, construct_url_with_subpath, get_response_body,
    get_response_headers, join_base_and_endpoint_path, response_handling, streaming_response,
    update_stream_param,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use rstest::rstest;
use serde_json::{Map, Value};

const HOP: &[(&str, &str)] = &[
    ("transfer-encoding", "chunked"),
    ("content-encoding", "gzip"),
    ("content-length", "9"),
    ("server", "up"),
    ("date", "d"),
    ("connection", "close"),
    ("keep-alive", "5"),
    ("x-up", "1"),
];

fn header_map(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.append(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    headers
}

#[rstest]
#[case::excluded(HOP, None, &[], &[("x-up", "1")])]
#[case::call_id(&[], Some("c1"), &[], &[("x-litellm-call-id", "c1")])]
#[case::empty_call_id(&[], Some(""), &[], &[])]
#[case::custom(&[("x-up", "1")], Some("c1"), HOP, &[("x-up", "1"), ("x-litellm-call-id", "c1")])]
#[case::custom_replaces(&[("x-up", "1"), ("x-up", "2")], None, &[("x-up", "3")], &[("x-up", "3")])]
fn passthrough_response_headers_behavior(
    #[case] upstream: &[(&'static str, &'static str)],
    #[case] call_id: Option<&str>,
    #[case] custom: &[(&'static str, &'static str)],
    #[case] expected: &[(&'static str, &'static str)],
) {
    let custom_headers = header_map(custom);
    let headers = get_response_headers(&header_map(upstream), call_id, Some(&custom_headers));
    assert_eq!(headers, header_map(expected));
}

#[test]
fn passthrough_response_headers_custom_values_cannot_restore_excluded_headers() {
    let upstream = header_map(&[("content-length", "9"), ("x-up", "upstream")]);
    let custom = header_map(&[
        ("content-length", "1"),
        ("server", "custom"),
        ("x-up", "custom"),
    ]);

    let headers = get_response_headers(&upstream, Some("call"), Some(&custom));

    assert!(!headers.contains_key("content-length"));
    assert!(!headers.contains_key("server"));
    assert_eq!(
        headers.get("x-up").and_then(|value| value.to_str().ok()),
        Some("custom")
    );
    assert_eq!(
        headers
            .get("x-litellm-call-id")
            .and_then(|value| value.to_str().ok()),
        Some("call")
    );
}

#[test]
fn passthrough_response_headers_reject_invalid_call_id() {
    let headers = get_response_headers(&HeaderMap::new(), Some("bad\nvalue"), None);

    assert!(!headers.contains_key("x-litellm-call-id"));
}

#[rstest]
#[case::plain("https://up/b", "v1/x", true, "https://up/b/v1/x")]
#[case::slashes("https://up/b/", "/v1/x/", true, "https://up/b/v1/x/")]
#[case::dots("https://up/b", "v1/../x/./y/", true, "https://up/b/x/y/")]
#[case::climb("https://up/b", "../../etc", true, "https://up/b/etc")]
#[case::root("https://up/b", "/", true, "https://up/b/")]
#[case::normalized_root("https://up/b", "v1/..", true, "https://up/b/")]
#[case::excluded("https://up/b", "v1", false, "https://up/b")]
#[case::empty("https://up/b", "", true, "https://up/b")]
fn passthrough_url_joining_behavior(
    #[case] base: &str,
    #[case] subpath: &str,
    #[case] include: bool,
    #[case] expected: &str,
) {
    assert_eq!(construct_url_with_subpath(base, subpath, include), expected);
}

#[rstest]
#[case::no_base("https://up", "v1/../chat/", "/chat/")]
#[case::no_base_root("https://up/", "/", "/")]
#[case::prefix("https://up/api/", "/v1/chat", "/api/v1/chat")]
#[case::trailing("https://up/api", "v1/", "/api/v1/")]
#[case::itself("https://up/api", "v1/..", "/api")]
#[case::climb_out("https://up/api", "../../x", "/api/")]
#[case::sibling("https://up/api", "../apix", "/api/")]
#[case::climb_back("https://up/api", "../../api/x", "/api/x")]
#[case::double_slash("https://up//api", "x", "//api/x")]
#[case::triple_slash("https://up///api", "x", "///api/")]
#[case::root_parent("https://up/", "../x", "/x")]
#[case::root_parent_with_trailing_slash("https://up/", "../x/", "/x/")]
#[case::base_parent_preserves_base("https://up/api", "../../", "/api/")]
#[case::base_parent_with_name("https://up/api", "../api/", "/api/")]
fn passthrough_url_joining_behavior_base(
    #[case] base: &str,
    #[case] endpoint: &str,
    #[case] expected: &str,
) {
    assert_eq!(join_base_and_endpoint_path(base, endpoint), expected);
}

#[rstest]
#[case::event_stream(500, "text/event-stream; charset=utf-8", ResponseHandling::Stream)]
#[case::error(500, "application/octet-stream", ResponseHandling::Buffer)]
#[case::bad_request(400, "text/plain", ResponseHandling::Buffer)]
#[case::json(200, "Application/JSON; charset=utf-8", ResponseHandling::Buffer)]
#[case::problem_json(200, "application/problem+json", ResponseHandling::Buffer)]
#[case::amz_json(200, "application/x-amz-json-1.1", ResponseHandling::Buffer)]
#[case::empty(200, "", ResponseHandling::Buffer)]
#[case::octet(399, "application/octet-stream", ResponseHandling::Relay)]
#[case::jsonl(200, "application/jsonl", ResponseHandling::Relay)]
#[case::json_suffix(
    200,
    "application/vnd.answer+json; charset=utf-8",
    ResponseHandling::Buffer
)]
#[case::amz_prefix(200, "application/x-amz-json-1.0", ResponseHandling::Buffer)]
#[case::plain(200, "text/plain", ResponseHandling::Relay)]
fn passthrough_buffering_behavior(
    #[case] status: u16,
    #[case] content_type: &'static str,
    #[case] expected: ResponseHandling,
) {
    let headers = header_map(&[("content-type", content_type)]);
    assert_eq!(response_handling(status, &headers), expected);
    assert_eq!(
        streaming_response(&headers),
        expected == ResponseHandling::Stream
    );
    assert_eq!(
        buffer_response(status, &headers),
        status >= 400 || expected == ResponseHandling::Buffer
    );
}

#[test]
fn passthrough_streaming_response_without_content_type_is_false() {
    assert!(!streaming_response(&HeaderMap::new()));
}

#[rstest]
#[case::object(br#"{"ok":true}"#, Some(serde_json::from_str::<Value>(r#"{"ok":true}"#).unwrap()))]
#[case::array(b"[1,2]", Some(serde_json::from_str::<Value>("[1,2]").unwrap()))]
#[case::null(b"null", Some(Value::Null))]
#[case::scalar(b"42", Some(Value::from(42_i64)))]
#[case::empty(b"", None)]
#[case::invalid(b"not-json", None)]
fn passthrough_response_body_behavior(#[case] body: &[u8], #[case] expected: Option<Value>) {
    assert_eq!(get_response_body(body), expected);
}

#[rstest]
#[case::body_wins(serde_json::from_str::<Value>(r#"{"stream":false}"#).unwrap(), Some(false))]
#[case::body_null(serde_json::from_str::<Value>(r#"{"stream":null}"#).unwrap(), None)]
#[case::fallback(serde_json::from_str::<Value>("{}").unwrap(), Some(true))]
#[case::non_boolean_falls_back(
    serde_json::from_str::<Value>(r#"{"stream":"yes"}"#).unwrap(),
    None
)]
fn passthrough_buffering_behavior_stream(
    #[case] body_value: Value,
    #[case] expected: Option<bool>,
) {
    let parsed_body: Map<String, Value> = serde_json::from_value(body_value).unwrap();
    assert_eq!(update_stream_param(&parsed_body, Some(true)), expected);
}
