use crate::guardrails::model_armor::streams::rewrite::apply_sanitized;

use super::support::value;

#[test]
fn apply_sanitized_replaces_every_non_empty_choice_message() {
    let mut body =
        value(r#"{"choices": [{"message": {"content": "a"}}, {"message": {"content": "b"}}]}"#);
    apply_sanitized(&mut body, "[REDACTED]");
    assert_eq!(
        body,
        value(
            r#"{"choices": [{"message": {"content": "[REDACTED]"}}, {"message": {"content": "[REDACTED]"}}]}"#
        )
    );
}

#[test]
fn apply_sanitized_leaves_an_empty_message_content_alone() {
    let mut body =
        value(r#"{"choices": [{"message": {"content": ""}}, {"message": {"content": "b"}}]}"#);
    apply_sanitized(&mut body, "[REDACTED]");
    assert_eq!(
        body,
        value(
            r#"{"choices": [{"message": {"content": ""}}, {"message": {"content": "[REDACTED]"}}]}"#
        )
    );
}

#[test]
fn apply_sanitized_leaves_a_null_content_alone() {
    let mut body = value(r#"{"choices": [{"message": {"content": null}}]}"#);
    apply_sanitized(&mut body, "[REDACTED]");
    assert_eq!(
        body,
        value(r#"{"choices": [{"message": {"content": null}}]}"#)
    );
}

#[test]
fn apply_sanitized_leaves_a_non_text_content_alone() {
    let mut body = value(r#"{"choices": [{"message": {"content": [{"type": "text"}]}}]}"#);
    apply_sanitized(&mut body, "[REDACTED]");
    assert_eq!(
        body,
        value(r#"{"choices": [{"message": {"content": [{"type": "text"}]}}]}"#)
    );
}

#[test]
fn apply_sanitized_without_choices_changes_nothing() {
    let mut body = value(r#"{"id": "chatcmpl-1"}"#);
    apply_sanitized(&mut body, "[REDACTED]");
    assert_eq!(body, value(r#"{"id": "chatcmpl-1"}"#));
}

#[test]
fn apply_sanitized_skips_a_choice_without_a_message() {
    let mut body = value(r#"{"choices": [{"index": 0}]}"#);
    apply_sanitized(&mut body, "[REDACTED]");
    assert_eq!(body, value(r#"{"choices": [{"index": 0}]}"#));
}

#[test]
fn apply_sanitized_skips_a_message_without_content() {
    let mut body = value(r#"{"choices": [{"message": {"role": "assistant"}}]}"#);
    apply_sanitized(&mut body, "[REDACTED]");
    assert_eq!(
        body,
        value(r#"{"choices": [{"message": {"role": "assistant"}}]}"#)
    );
}
