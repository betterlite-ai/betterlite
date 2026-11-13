#![cfg(test)]

use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use super::{as_object_list, as_str_object_dict, safe_response_text};
use crate::typesafe::DEFAULT_RESPONSE_TEXT_LIMIT;
use crate::typesafe::fixtures::json;

#[test]
fn as_str_object_dict_accepts_an_object() {
    let value = json(r#"{"role":"tool"}"#);
    let fields = as_str_object_dict(&value).expect("an object");
    assert_eq!(fields.get("role").and_then(Json::as_str), Some("tool"));
}

#[rstest]
#[case::null("null")]
#[case::boolean("true")]
#[case::number("3")]
#[case::text(r#""x""#)]
#[case::list("[]")]
fn as_str_object_dict_rejects_non_objects(#[case] text: &str) {
    assert!(as_str_object_dict(&json(text)).is_none());
}

#[test]
fn as_object_list_accepts_a_list() {
    let value = json(r#"[{"a":1},"b"]"#);
    let items = as_object_list(&value).expect("a list");
    assert_eq!(items.len(), 2);
}

#[rstest]
#[case::null("null")]
#[case::boolean("false")]
#[case::number("1")]
#[case::text(r#""x""#)]
#[case::object("{}")]
fn as_object_list_rejects_non_lists(#[case] text: &str) {
    assert!(as_object_list(&json(text)).is_none());
}

#[test]
fn safe_response_text_returns_empty_without_a_body() {
    assert_eq!(safe_response_text(None, 10), "");
}

#[test]
fn safe_response_text_decodes_a_body() {
    assert_eq!(
        safe_response_text(Some(br#"{"error":"boom"}"#), 500),
        r#"{"error":"boom"}"#
    );
}

#[test]
fn safe_response_text_truncates_by_characters_not_bytes() {
    assert_eq!(
        safe_response_text(Some("\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}".as_bytes()), 3),
        "\u{e9}\u{e9}\u{e9}"
    );
}

#[test]
fn safe_response_text_marks_a_body_that_is_not_utf8() {
    assert_eq!(
        safe_response_text(Some(&[0xff, 0xfe]), 500),
        "<undecodable response body>"
    );
    assert_eq!(
        safe_response_text(Some(&[0xff]), 0),
        "<undecodable response body>"
    );
}

#[test]
fn safe_response_text_default_limit_is_five_hundred_characters() {
    assert_eq!(DEFAULT_RESPONSE_TEXT_LIMIT, 500);
    let body = "a".repeat(600);
    let truncated = safe_response_text(Some(body.as_bytes()), DEFAULT_RESPONSE_TEXT_LIMIT);
    assert_eq!(truncated, "a".repeat(500));
}
