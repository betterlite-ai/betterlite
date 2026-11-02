#![cfg(test)]

use betterlite_ocr::json::Json;
use betterlite_ocr::request_format::RequestFormat;

use super::select;

#[test]
fn returns_none_when_litellm_format_was_requested() {
    let payload = Json::Text("native".to_owned());
    assert_eq!(
        select(Some(RequestFormat::Litellm), Some(&payload), &[]),
        None
    );
}

#[test]
fn returns_none_when_no_payload_was_captured() {
    assert_eq!(select(Some(RequestFormat::Native), None, &[]), None);
}

#[test]
fn returns_none_when_no_format_was_requested() {
    let payload = Json::Text("native".to_owned());
    assert_eq!(select(None, Some(&payload), &[]), None);
}

#[test]
fn strips_content_length_and_content_type_headers() {
    let payload = Json::Text("native".to_owned());
    let headers = vec![
        ("Content-Length".to_owned(), "42".to_owned()),
        ("Content-Type".to_owned(), "application/json".to_owned()),
        ("x-litellm-call-id".to_owned(), "abc".to_owned()),
    ];
    let native = select(Some(RequestFormat::Native), Some(&payload), &headers).unwrap();
    assert_eq!(native.payload, payload);
    assert_eq!(
        native.headers,
        vec![("x-litellm-call-id".to_owned(), "abc".to_owned())]
    );
}
