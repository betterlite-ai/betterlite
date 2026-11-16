use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::json;
use crate::fields::response::ServedTexts;
use crate::logging_streams::overlay::overlay_served_output_texts;

fn texts(values: &[Option<&str>]) -> ServedTexts {
    values
        .iter()
        .map(|value| value.map(str::to_owned))
        .collect()
}

fn overlay(response: &Json, served: &ServedTexts) -> Json {
    overlay_served_output_texts(response, Some(served))
}

fn test_overlays_texts_into_their_choices() {
    let response = json(
        r#"{"id":"chatcmpl-1","choices":[{"index":0,"message":{"role":"assistant","content":"old"}},{"index":1,"message":{"role":"assistant","content":"old"}}]}"#,
    );
    let overlaid = overlay(&response, &texts(&[Some("new"), None]));
    assert_eq!(
        overlaid,
        json(
            r#"{"id":"chatcmpl-1","choices":[{"index":0,"message":{"role":"assistant","content":"new"}},{"index":1,"message":{"role":"assistant","content":"old"}}]}"#,
        )
    );
}

fn test_keeps_a_response_without_texts() {
    let response = json(r#"{"choices":[{"message":{"content":"old"}}]}"#);
    assert_eq!(overlay_served_output_texts(&response, None), response);
}

fn test_keeps_a_response_that_is_not_an_object() {
    let response = json(r#"[{"message":{"content":"old"}}]"#);
    assert_eq!(overlay(&response, &texts(&[Some("new")])), response);
}

fn test_keeps_a_response_without_choices() {
    let response = json(r#"{"id":"chatcmpl-1"}"#);
    assert_eq!(overlay(&response, &texts(&[Some("new")])), response);
    let not_a_list = json(r#"{"choices":{}}"#);
    assert_eq!(overlay(&not_a_list, &texts(&[Some("new")])), not_a_list);
}

fn test_keeps_a_choice_that_is_not_an_object() {
    let response = json(r#"{"choices":["raw",{"message":{"role":"assistant","content":"old"}}]}"#);
    let overlaid = overlay(&response, &texts(&[Some("new"), Some("new")]));
    assert_eq!(
        overlaid,
        json(r#"{"choices":["raw",{"message":{"role":"assistant","content":"new"}}]}"#,)
    );
}

fn test_keeps_a_message_that_is_not_an_object() {
    let response = json(r#"{"choices":[{"message":"text"}]}"#);
    assert_eq!(overlay(&response, &texts(&[Some("new")])), response);
}

fn test_keeps_a_choice_without_a_message() {
    let response = json(r#"{"choices":[{"index":0}]}"#);
    assert_eq!(overlay(&response, &texts(&[Some("new")])), response);
}

fn test_keeps_content_that_already_matches() {
    let response = json(r#"{"choices":[{"message":{"role":"assistant","content":"same"}}]}"#);
    assert_eq!(overlay(&response, &texts(&[Some("same")])), response);
}

fn test_replaces_missing_content_with_the_served_text() {
    let response = json(r#"{"choices":[{"message":{"role":"assistant"}}]}"#);
    assert_eq!(
        overlay(&response, &texts(&[Some("new")])),
        json(r#"{"choices":[{"message":{"role":"assistant","content":"new"}}]}"#)
    );
}

fn test_keeps_an_empty_served_text_as_a_replacement() {
    let response = json(r#"{"choices":[{"message":{"content":"old"}}]}"#);
    assert_eq!(
        overlay(&response, &texts(&[Some("")])),
        json(r#"{"choices":[{"message":{"content":""}}]}"#)
    );
}

fn test_ignores_texts_past_the_choices() {
    let response = json(r#"{"choices":[{"message":{"content":"old"}}]}"#);
    assert_eq!(
        overlay(&response, &texts(&[Some("new"), Some("extra")])),
        json(r#"{"choices":[{"message":{"content":"new"}}]}"#)
    );
}

fn test_leaves_choices_past_the_texts() {
    let response =
        json(r#"{"choices":[{"message":{"content":"old"}},{"message":{"content":"later"}}]}"#);
    assert_eq!(
        overlay(&response, &texts(&[Some("new")])),
        json(r#"{"choices":[{"message":{"content":"new"}},{"message":{"content":"later"}}]}"#,)
    );
}

#[rstest]
#[case::overlay_replaces_texts_in_their_choices(test_overlays_texts_into_their_choices)]
#[case::overlay_keeps_a_response_without_texts(test_keeps_a_response_without_texts)]
#[case::overlay_keeps_a_response_that_is_not_an_object(test_keeps_a_response_that_is_not_an_object)]
#[case::overlay_keeps_a_response_without_choices(test_keeps_a_response_without_choices)]
#[case::overlay_keeps_a_choice_that_is_not_an_object(test_keeps_a_choice_that_is_not_an_object)]
#[case::overlay_keeps_a_message_that_is_not_an_object(test_keeps_a_message_that_is_not_an_object)]
#[case::overlay_keeps_a_choice_without_a_message(test_keeps_a_choice_without_a_message)]
#[case::overlay_keeps_content_that_already_matches(test_keeps_content_that_already_matches)]
#[case::overlay_replaces_missing_content(test_replaces_missing_content_with_the_served_text)]
#[case::overlay_keeps_an_empty_served_text(test_keeps_an_empty_served_text_as_a_replacement)]
#[case::overlay_ignores_texts_past_the_choices(test_ignores_texts_past_the_choices)]
#[case::overlay_leaves_choices_past_the_texts(test_leaves_choices_past_the_texts)]
fn logging_streams_behavior_tests_behavior(#[case] behavior: fn()) {
    behavior();
}
