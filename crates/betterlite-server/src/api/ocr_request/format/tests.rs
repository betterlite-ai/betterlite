#![cfg(test)]

use betterlite_ocr::json::{Json, Object};
use betterlite_ocr::request_format::RequestFormat;

use super::resolve;

#[test]
fn body_value_wins_over_header() {
    let mut fields = Object::new();
    fields.insert("req_format".to_owned(), Json::Text("native".to_owned()));
    let (remaining, format) = resolve(fields, Some("litellm")).unwrap();
    assert_eq!(format, Some(RequestFormat::Native));
    assert!(!remaining.contains_key("req_format"));
}

#[test]
fn header_is_used_when_body_omits_req_format() {
    let fields = Object::new();
    let (_, format) = resolve(fields, Some("  LITELLM  ")).unwrap();
    assert_eq!(format, Some(RequestFormat::Litellm));
}

#[test]
fn neither_body_nor_header_leaves_format_unset() {
    let fields = Object::new();
    let (_, format) = resolve(fields, None).unwrap();
    assert_eq!(format, None);
}

#[test]
fn invalid_body_value_is_rejected() {
    let mut fields = Object::new();
    fields.insert("req_format".to_owned(), Json::Text("yaml".to_owned()));
    resolve(fields, None).unwrap_err();
}

#[test]
fn null_body_value_falls_back_to_header() {
    let mut fields = Object::new();
    fields.insert("req_format".to_owned(), Json::Null);
    let (remaining, format) = resolve(fields, Some("native")).unwrap();
    assert_eq!(format, Some(RequestFormat::Native));
    assert!(!remaining.contains_key("req_format"));
}

#[test]
fn null_body_value_without_header_leaves_format_unset() {
    let mut fields = Object::new();
    fields.insert("req_format".to_owned(), Json::Null);
    let (remaining, format) = resolve(fields, None).unwrap();
    assert_eq!(format, None);
    assert!(!remaining.contains_key("req_format"));
}
