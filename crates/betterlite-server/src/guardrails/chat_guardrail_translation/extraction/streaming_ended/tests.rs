#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::check_streaming_has_ended;
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{json, stream_items};

#[rstest]
#[case::no_chunks(vec![], false)]
#[case::finished(vec![json(r#"{"choices": [{"finish_reason": "stop"}]}"#)], true)]
#[case::any_choice_counts(
    vec![json(r#"{"choices": [{"finish_reason": null}, {"finish_reason": "stop"}]}"#)],
    true,
)]
#[case::finish_in_the_last_chunk(
    vec![json(r#"{"choices": [{"delta": {"content": "a"}}]}"#), json(r#"{"choices": [{"finish_reason": "stop"}]}"#)],
    true,
)]
#[case::null_finish_reason(vec![json(r#"{"choices": [{"finish_reason": null}]}"#)], false)]
#[case::absent_finish_reason(vec![json(r#"{"choices": [{"delta": {"content": "a"}}]}"#)], false)]
#[case::usage_only_chunk(vec![json(r#"{"choices": [], "usage": {"total_tokens": 3}}"#)], false)]
fn the_stream_has_ended_once_a_choice_carries_a_finish_reason(
    #[case] chunks: Vec<JsonValue>,
    #[case] expected: bool,
) {
    let items = stream_items(chunks);
    assert_eq!(check_streaming_has_ended(&items), expected);
}
