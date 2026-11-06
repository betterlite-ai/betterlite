#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::{
    assistant_text_from_response, last_text_by_role, normalize_message, synthesize_user_message,
};
use crate::xecguard_primitives::wire::XecGuardMessage;

fn value(wire: &str) -> GuardrailValue {
    serde_json::from_str(wire).expect("json value")
}

fn message(role: &str, content: &str) -> XecGuardMessage {
    XecGuardMessage {
        role: role.to_owned(),
        content: content.to_owned(),
    }
}

#[test]
fn message_normalization_flattens_multimodal_content_to_text() {
    let normalized = normalize_message(&value(
        r#"{"role":"user","content":[{"type":"text","text":"first"},{"type":"image_url","image_url":{"url":"x"}},{"type":"text","text":"second"}]}"#,
    ))
    .expect("object message");
    assert_eq!(normalized, message("user", "first\nsecond"));
}

#[rstest]
#[case::empty_text_parts(
    r#"{"role":"user","content":[{"type":"text","text":"a"},{"type":"text","text":""}]}"#,
    "a\n"
)]
#[case::missing_content(r#"{"role":"user"}"#, "")]
#[case::null_content(r#"{"role":"user","content":null}"#, "")]
#[case::object_content(r#"{"role":"user","content":{"type":"text"}}"#, "")]
#[case::number_content(r#"{"role":"user","content":7}"#, "")]
fn message_normalization_handles_empty_and_missing_content(
    #[case] wire: &str,
    #[case] expected: &str,
) {
    let normalized = normalize_message(&value(wire)).expect("object message");
    assert_eq!(normalized.content, expected);
}

#[test]
fn message_normalization_defaults_a_missing_role_to_user() {
    let normalized = normalize_message(&value(r#"{"content":"hello"}"#)).expect("object message");
    assert_eq!(normalized, message("user", "hello"));
    assert_eq!(normalize_message(&value("[1,2]")), None);
}

#[test]
fn message_synthesis_joins_only_non_empty_texts() {
    assert_eq!(
        synthesize_user_message(&["a".to_owned(), String::new(), "b".to_owned()]),
        Some(message("user", "a\nb"))
    );
    assert_eq!(synthesize_user_message(&[String::new()]), None);
    assert_eq!(synthesize_user_message(&[]), None);
}

#[test]
fn message_last_text_by_role_stops_at_the_newest_matching_message() {
    let messages = [
        message("user", "old"),
        message("assistant", "answer"),
        message("user", ""),
    ];
    assert_eq!(last_text_by_role(&messages, "user"), None);
    assert_eq!(
        last_text_by_role(&messages, "assistant"),
        Some("answer".to_owned())
    );
    assert_eq!(last_text_by_role(&messages, "system"), None);
}

#[test]
fn message_assistant_text_joins_every_choice_with_text() {
    let response = value(
        r#"{"choices":[{"message":{"content":"first"}},{"message":{"content":[{"type":"text","text":"second"},{"type":"text","text":"third"}]}},{"message":{"content":null}},{"message":{}},{"delta":{"content":"ignored"}}]}"#,
    );
    assert_eq!(
        assistant_text_from_response(&response),
        Some("first\nsecond\nthird".to_owned())
    );
    assert_eq!(assistant_text_from_response(&value("{}")), None);
    assert_eq!(
        assistant_text_from_response(&value(r#"{"choices":[]}"#)),
        None
    );
    assert_eq!(
        assistant_text_from_response(&value(r#"{"choices":[{"message":{"content":""}}]}"#)),
        None
    );
    assert_eq!(assistant_text_from_response(&value("[]")), None);
}

#[test]
fn message_assistant_text_flattens_list_content_and_skips_blank_parts() {
    let response = value(
        r#"{"choices":[{"message":{"content":[{"type":"text","text":""},{"type":"text","text":"kept"},{"type":"image_url","image_url":{}},{"type":"text","text":""}]}}]}"#,
    );
    assert_eq!(
        assistant_text_from_response(&response),
        Some("kept".to_owned())
    );
}
