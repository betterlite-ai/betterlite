#![cfg(test)]

use super::content::chat_content;
use super::support::value;

#[test]
fn chat_content_returns_the_first_choices_message() {
    let response = value(
        r#"{"choices": [{"message": {"role": "assistant", "content": "one"}}, {"message": {"role": "assistant", "content": "two"}}]}"#,
    );
    assert_eq!(
        chat_content(&response),
        Some(value(r#"{"role": "assistant", "content": "one"}"#))
    );
}

#[test]
fn chat_content_is_none_without_choices() {
    assert_eq!(chat_content(&value(r#"{"choices": []}"#)), None);
    assert_eq!(chat_content(&value(r#"{"id": "chatcmpl-1"}"#)), None);
}

#[test]
fn chat_content_is_none_for_a_missing_message() {
    assert_eq!(chat_content(&value(r#"{"choices": [{"index": 0}]}"#)), None);
    assert_eq!(
        chat_content(&value(r#"{"choices": [{"message": null}]}"#)),
        None
    );
}
