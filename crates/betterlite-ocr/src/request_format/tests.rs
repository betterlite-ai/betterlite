#![cfg(test)]

use super::{RequestFormat, parse_ocr_request_format};
use crate::json::Json;

#[test]
fn parse_ocr_request_format_accepts_litellm() {
    let value = Json::Text("litellm".to_owned());
    assert_eq!(
        parse_ocr_request_format(&value).unwrap(),
        RequestFormat::Litellm
    );
}

#[test]
fn parse_ocr_request_format_accepts_native() {
    let value = Json::Text("native".to_owned());
    assert_eq!(
        parse_ocr_request_format(&value).unwrap(),
        RequestFormat::Native
    );
}

#[test]
fn parse_ocr_request_format_strips_and_lowercases() {
    let value = Json::Text("  NATIVE  ".to_owned());
    assert_eq!(
        parse_ocr_request_format(&value).unwrap(),
        RequestFormat::Native
    );
}

#[test]
fn parse_ocr_request_format_rejects_unknown_string() {
    let value = Json::Text("yaml".to_owned());
    parse_ocr_request_format(&value).unwrap_err();
}

#[test]
fn parse_ocr_request_format_rejects_non_string() {
    let value = Json::Number(serde_json::Number::from(5_i64));
    parse_ocr_request_format(&value).unwrap_err();
}
