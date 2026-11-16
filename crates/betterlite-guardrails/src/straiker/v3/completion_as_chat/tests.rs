#![cfg(test)]

use rstest::rstest;

use super::{TextCompletionResponse, text_completion_as_chat};
use crate::straiker::v3::fixtures::{as_json, parsed};

#[rstest]
#[case::one_choice(
    r#"{"id":"cmpl-1","created":1700000000,"model":"davinci","choices":[{"finish_reason":"stop","text":"done"}]}"#,
    r#"{"id":"cmpl-1","object":"chat.completion","created":1700000000,"model":"davinci","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"done"}}]}"#
)]
#[case::choices_are_indexed_in_order(
    r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"length","text":"a"},{"finish_reason":null,"text":"b"}]}"#,
    r#"{"id":"c","object":"chat.completion","created":1,"model":"m","choices":[{"index":0,"finish_reason":"length","message":{"role":"assistant","content":"a"}},{"index":1,"finish_reason":null,"message":{"role":"assistant","content":"b"}}]}"#
)]
#[case::missing_text_is_empty_content(
    r#"{"id":"c","created":1,"model":"m","choices":[{"finish_reason":"stop"}]}"#,
    r#"{"id":"c","object":"chat.completion","created":1,"model":"m","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":""}}]}"#
)]
#[case::usage_is_carried(
    r#"{"id":"c","created":1,"model":"m","choices":[],"usage":{"prompt_tokens":3,"completion_tokens":4}}"#,
    r#"{"id":"c","object":"chat.completion","created":1,"model":"m","choices":[],"usage":{"prompt_tokens":3,"completion_tokens":4}}"#
)]
#[case::empty_usage_is_left_out(
    r#"{"id":"c","created":1,"model":"m","choices":[],"usage":{}}"#,
    r#"{"id":"c","object":"chat.completion","created":1,"model":"m","choices":[]}"#
)]
#[case::unknown_model_is_null(
    r#"{"id":"c","created":1,"choices":[]}"#,
    r#"{"id":"c","object":"chat.completion","created":1,"model":null,"choices":[]}"#
)]
fn presents_completion_as_chat(#[case] source: &str, #[case] expected: &str) {
    let response: TextCompletionResponse = serde_json::from_str(source).unwrap();
    let chat = text_completion_as_chat(&response);
    assert_eq!(as_json(&chat), parsed(expected));
    assert_eq!(
        chat.keys()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .first()
            .copied(),
        Some("id")
    );
}
