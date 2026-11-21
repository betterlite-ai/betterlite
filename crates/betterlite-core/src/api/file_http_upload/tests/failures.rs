use betterlite_models::decisions::Json;
use http::header::{CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue};

use crate::api::file_http_upload::error::status_failure;

// httpx re-encodes the decoded body, so the old length and encoding go and the other headers stay
#[test]
fn status_failure_keeps_headers_and_recomputes_length() {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    headers.insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    headers.insert(CONTENT_LENGTH, HeaderValue::from_static("123"));
    let facts = status_failure(400, b"bad media", &headers);
    assert_eq!(facts.response_status, Some(400));
    assert_eq!(facts.text.as_deref(), Some("bad media"));
    let response = facts.response.expect("the response facts");
    assert_eq!(response.status_code, 400);
    let kept = response.headers.expect("the kept headers");
    assert_eq!(
        kept.get("content-type"),
        Some(&Json::Text(String::from("text/plain")))
    );
    assert_eq!(
        kept.get("content-length"),
        Some(&Json::Text(String::from("9")))
    );
    assert_eq!(kept.get("content-encoding"), None);
}

#[test]
fn status_failure_omits_content_length_for_an_empty_body() {
    let facts = status_failure(400, b"", &HeaderMap::new());
    let response = facts.response.expect("the response facts");
    assert_eq!(
        response
            .headers
            .expect("the kept headers")
            .get("content-length"),
        None
    );
}
