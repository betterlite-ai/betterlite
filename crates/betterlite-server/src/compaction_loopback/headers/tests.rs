#![cfg(test)]

use axum::http::HeaderMap;
use axum::http::header::{AUTHORIZATION, HeaderName, HeaderValue};
use rstest::rstest;

use super::child_headers;

#[test]
fn the_five_transport_headers_are_dropped_in_any_letter_case() {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer sk-test"));
    // from_static rejects uppercase custom names, so the wire bytes go through from_bytes
    headers.insert(
        HeaderName::from_bytes(b"Content-Length").unwrap(),
        HeaderValue::from_static("42"),
    );
    headers.insert(
        HeaderName::from_bytes(b"X-LiteLLM-Call-Id").unwrap(),
        HeaderValue::from_static("call-1"),
    );
    headers.insert(
        HeaderName::from_bytes(b"X-LiteLLM-Num-Retries").unwrap(),
        HeaderValue::from_static("2"),
    );
    headers.insert(
        HeaderName::from_bytes(b"X-LiteLLM-Timeout").unwrap(),
        HeaderValue::from_static("30"),
    );
    headers.insert(
        HeaderName::from_bytes(b"X-LiteLLM-Stream-Timeout").unwrap(),
        HeaderValue::from_static("60"),
    );
    let child = child_headers(&headers, false);
    let names: Vec<&str> = child.keys().map(HeaderName::as_str).collect();
    assert_eq!(names, vec!["authorization"]);
}

#[rstest]
#[case("litellm-disable-message-redaction")]
#[case("Litellm-Disable-Message-Redaction")]
#[test]
fn the_redaction_opt_out_follows_the_logging_flag(#[case] name: &'static str) {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer sk-test"));
    headers.insert(
        HeaderName::from_bytes(name.as_bytes()).unwrap(),
        HeaderValue::from_static("true"),
    );
    let kept = child_headers(&headers, false);
    let kept_names: Vec<&str> = kept.keys().map(HeaderName::as_str).collect();
    assert_eq!(
        kept_names,
        vec!["authorization", "litellm-disable-message-redaction"]
    );
    let dropped = child_headers(&headers, true);
    let dropped_names: Vec<&str> = dropped.keys().map(HeaderName::as_str).collect();
    assert_eq!(dropped_names, vec!["authorization"]);
    assert_eq!(dropped.get(AUTHORIZATION).unwrap(), "Bearer sk-test");
}

#[test]
fn the_other_headers_stay_in_order_with_their_repeats() {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer sk-test"));
    headers.append(
        HeaderName::from_static("x-request-extra"),
        HeaderValue::from_static("one"),
    );
    headers.append(
        HeaderName::from_static("x-request-extra"),
        HeaderValue::from_static("two"),
    );
    headers.insert(
        HeaderName::from_static("content-length"),
        HeaderValue::from_static("42"),
    );
    headers.insert(
        HeaderName::from_static("x-litellm-timeout"),
        HeaderValue::from_static("30"),
    );
    headers.insert(
        HeaderName::from_static("litellm-disable-message-redaction"),
        HeaderValue::from_static("true"),
    );
    let child = child_headers(&headers, false);
    let sent: Vec<(&str, &[u8])> = child
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_bytes()))
        .collect();
    assert_eq!(
        sent,
        vec![
            ("authorization", b"Bearer sk-test".as_slice()),
            ("x-request-extra", b"one".as_slice()),
            ("x-request-extra", b"two".as_slice()),
            ("litellm-disable-message-redaction", b"true".as_slice()),
        ]
    );
}
