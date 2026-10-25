#![cfg(test)]
use super::{buffer_response, construct_url_with_subpath, get_response_headers};
use reqwest::header::{HeaderMap, HeaderValue};
#[test]
fn passthrough_response_headers_behavior() {
    let mut headers = HeaderMap::new();
    headers.insert("content-length", HeaderValue::from_static("x"));
    assert!(!get_response_headers(&headers, Some("id"), None).contains_key("content-length"));
}
#[test]
fn passthrough_url_joining_behavior() {
    assert_eq!(
        construct_url_with_subpath("https://upstream/base", "v1/../items/", true),
        "https://upstream/base/items/"
    );
}
#[test]
fn passthrough_buffering_behavior() {
    let headers = HeaderMap::new();
    assert!(buffer_response(500, &headers));
}
