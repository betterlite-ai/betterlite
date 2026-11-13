#![cfg(test)]

use betterlite_models::decisions::Json;

use super::content::text_completion_content;
use super::support::value;

#[test]
fn text_content_returns_the_first_choice_text() {
    let response = value(r#"{"choices": [{"text": "one"}, {"text": "two"}]}"#);
    assert_eq!(
        text_completion_content(&response),
        Some(Json::Text("one".to_owned()))
    );
}

#[test]
fn text_content_is_none_without_choices() {
    assert_eq!(text_completion_content(&value(r#"{"choices": []}"#)), None);
    assert_eq!(text_completion_content(&value(r#"{"id": "cmpl-1"}"#)), None);
}

#[test]
fn text_content_is_none_for_a_null_text() {
    assert_eq!(
        text_completion_content(&value(r#"{"choices": [{"text": null}]}"#)),
        None
    );
}
