#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::first_choice_has_finished;
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{json, stream_items};

#[rstest]
#[case::no_chunks(vec![], false)]
#[case::first_choice_finished(vec![json(r#"{"choices": [{"finish_reason": "stop"}]}"#)], true)]
#[case::finish_in_a_later_chunk(
    vec![json(r#"{"choices": [{"finish_reason": null}]}"#), json(r#"{"choices": [{"finish_reason": "length"}]}"#)],
    true,
)]
#[case::null_finish_reason(vec![json(r#"{"choices": [{"finish_reason": null}]}"#)], false)]
#[case::absent_finish_reason(vec![json(r#"{"choices": [{"delta": {}}]}"#)], false)]
#[case::empty_string_is_a_finish_reason(vec![json(r#"{"choices": [{"finish_reason": ""}]}"#)], true)]
#[case::only_the_second_choice_finished(
    vec![json(r#"{"choices": [{"finish_reason": null}, {"finish_reason": "stop"}]}"#)],
    false,
)]
#[case::chunk_without_choices_is_skipped(
    vec![json(r#"{"choices": []}"#), json(r#"{"id": "x"}"#), json(r#"{"choices": [{"finish_reason": "stop"}]}"#)],
    true,
)]
fn a_finish_on_the_first_choice_ends_the_stream(
    #[case] chunks: Vec<JsonValue>,
    #[case] expected: bool,
) {
    let items = stream_items(chunks);
    assert_eq!(first_choice_has_finished(&items), expected);
}
