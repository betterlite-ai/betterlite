#![cfg(test)]

use super::{
    construct_target_url_with_subpath, get_response_headers, should_buffer_passthrough_response,
};
use reqwest::header::{HeaderMap, HeaderValue};

#[test]
fn passthrough_url_joining_behavior() {
    assert_eq!(
        construct_target_url_with_subpath("https://upstream/base", "v1/../items/", true),
        "https://upstream/base/items/"
    );
}

#[test]
fn passthrough_response_headers_behavior() {
    let mut headers = HeaderMap::new();
    headers.insert("content-length", HeaderValue::from_static("10"));
    let output = get_response_headers(&headers, Some("call-1"), None);
    assert!(!output.contains_key("content-length"));
    assert_eq!(output.get("x-litellm-call-id").unwrap(), "call-1");
}

#[test]
fn passthrough_buffering_behavior() {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    assert!(should_buffer_passthrough_response(200, &headers));
}
