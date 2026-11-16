#![cfg(test)]

use rstest::rstest;
use serde_json::Value;

use super::{ReplyFailure, reply_error};

fn fixture(text: &str) -> Value {
    serde_json::from_str(text).expect("test fixture must be valid JSON")
}

#[test]
fn ignores_a_reply_without_an_error() {
    assert!(reply_error(&fixture(r#"{"data": []}"#)).is_none());
}

#[test]
fn ignores_a_null_error() {
    assert!(reply_error(&fixture(r#"{"error": null}"#)).is_none());
}

#[rstest]
#[case::text(r#""boom""#)]
#[case::empty_list("[]")]
#[case::nonempty_list("[1]")]
#[case::zero("0")]
#[case::false_flag("false")]
#[case::true_flag("true")]
#[case::number("5")]
fn a_non_object_error_is_a_422_without_a_message(#[case] error: &str) {
    let reply = fixture(&format!(r#"{{"error": {error}}}"#));
    assert_eq!(
        reply_error(&reply),
        Some(ReplyFailure {
            status: 422,
            message: None,
        })
    );
}

#[test]
fn ignores_an_empty_string_error() {
    assert!(reply_error(&fixture(r#"{"error": ""}"#)).is_none());
}

#[test]
fn ignores_an_empty_error_object() {
    assert!(reply_error(&fixture(r#"{"error": {}}"#)).is_none());
}

#[test]
fn ignores_a_null_message_with_a_null_code() {
    assert!(reply_error(&fixture(r#"{"error": {"message": null, "code": null}}"#)).is_none());
}

#[rstest]
#[case::empty_string(r#""""#)]
#[case::zero("0")]
#[case::false_flag("false")]
#[case::empty_list("[]")]
#[case::empty_object("{}")]
#[case::null("null")]
fn ignores_a_blank_message_without_a_code(#[case] message: &str) {
    let reply = fixture(&format!(r#"{{"error": {{"message": {message}}}}}"#));
    assert!(reply_error(&reply).is_none());
}

#[test]
fn raises_a_meaningful_message_without_a_code_as_422() {
    assert_eq!(
        reply_error(&fixture(r#"{"error": {"message": "boom"}}"#)),
        Some(ReplyFailure {
            status: 422,
            message: Some(String::from("boom")),
        })
    );
}

#[test]
fn raises_a_code_alone_without_a_message() {
    assert_eq!(
        reply_error(&fixture(r#"{"error": {"code": 429}}"#)),
        Some(ReplyFailure {
            status: 429,
            message: None,
        })
    );
}

#[test]
fn keeps_an_empty_message_when_a_code_is_present() {
    assert_eq!(
        reply_error(&fixture(r#"{"error": {"message": "", "code": 429}}"#)),
        Some(ReplyFailure {
            status: 429,
            message: Some(String::new()),
        })
    );
}

#[test]
fn keeps_no_message_for_a_null_message_with_a_code() {
    assert_eq!(
        reply_error(&fixture(r#"{"error": {"message": null, "code": 429}}"#)),
        Some(ReplyFailure {
            status: 429,
            message: None,
        })
    );
}

#[rstest]
#[case::text(r#""rate_limit""#)]
#[case::float("1.5")]
#[case::negative("-1")]
#[case::too_large("70000")]
fn a_code_that_is_not_a_status_falls_back_to_422(#[case] code: &str) {
    let reply = fixture(&format!(
        r#"{{"error": {{"message": "boom", "code": {code}}}}}"#
    ));
    assert_eq!(
        reply_error(&reply),
        Some(ReplyFailure {
            status: 422,
            message: Some(String::from("boom")),
        })
    );
}

#[test]
fn encodes_a_structured_message_as_json() {
    assert_eq!(
        reply_error(&fixture(r#"{"error": {"message": {"reason": "boom"}}}"#)),
        Some(ReplyFailure {
            status: 422,
            message: Some(String::from(r#"{"reason":"boom"}"#)),
        })
    );
}

#[rstest]
#[case::true_flag("true")]
#[case::number("5")]
fn a_truthy_message_that_is_not_a_string_is_its_json_text(#[case] message: &str) {
    let reply = fixture(&format!(r#"{{"error": {{"message": {message}}}}}"#));
    assert_eq!(
        reply_error(&reply),
        Some(ReplyFailure {
            status: 422,
            message: Some(String::from(message)),
        })
    );
}
