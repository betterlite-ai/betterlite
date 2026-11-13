#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::apply_input_texts;
use crate::guardrails::chat_guardrail_translation::extraction::apply_error::ApplyError;
use crate::guardrails::chat_guardrail_translation::extraction::extracted_content::TextMapping;
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{json, values};

fn rewrite(
    source: JsonValue,
    replies: &[&str],
    mappings: &[TextMapping],
) -> (Result<(), ApplyError>, JsonValue) {
    let mut messages = values(source);
    let responses: Vec<String> = replies.iter().map(|text| (*text).to_owned()).collect();
    let result = apply_input_texts(&mut messages, &responses, mappings);
    (result, serde_json::to_value(&messages).unwrap())
}

#[rstest]
#[case::string_content(
    json(r#"[{"role": "user", "content": "secret"}]"#),
    &["clean"],
    &[(0, None)],
    json(r#"[{"role": "user", "content": "clean"}]"#),
)]
#[case::list_item_text_keeps_siblings(
    json(r#"[{"role": "user", "content": [{"type": "text", "text": "secret"}, {"type": "image_url", "image_url": {"url": "u"}}]}]"#),
    &["clean"],
    &[(0, Some(0))],
    json(r#"[{"role": "user", "content": [{"type": "text", "text": "clean"}, {"type": "image_url", "image_url": {"url": "u"}}]}]"#),
)]
#[case::list_item_after_an_image(
    json(r#"[{"content": [{"type": "image_url"}, {"text": "secret"}]}]"#),
    &["clean"],
    &[(0, Some(1))],
    json(r#"[{"content": [{"type": "image_url"}, {"text": "clean"}]}]"#),
)]
#[case::each_mapping_picks_its_message(
    json(r#"[{"content": "one"}, {"content": "two"}, {"content": "three"}]"#),
    &["a", "c"],
    &[(0, None), (2, None)],
    json(r#"[{"content": "a"}, {"content": "two"}, {"content": "c"}]"#),
)]
#[case::fewer_responses_than_mappings(
    json(r#"[{"content": "one"}, {"content": "two"}]"#),
    &["a"],
    &[(0, None), (1, None)],
    json(r#"[{"content": "a"}, {"content": "two"}]"#),
)]
#[case::string_content_with_a_list_slot_is_untouched(
    json(r#"[{"content": "keep"}]"#),
    &["clean"],
    &[(0, Some(0))],
    json(r#"[{"content": "keep"}]"#),
)]
#[case::list_content_with_a_string_slot_is_untouched(
    json(r#"[{"content": [{"text": "keep"}]}]"#),
    &["clean"],
    &[(0, None)],
    json(r#"[{"content": [{"text": "keep"}]}]"#),
)]
#[case::null_content_is_skipped(
    json(r#"[{"role": "assistant", "content": null}]"#),
    &["clean"],
    &[(0, None)],
    json(r#"[{"role": "assistant", "content": null}]"#),
)]
#[case::absent_content_is_not_created(
    json(r#"[{"role": "assistant"}]"#),
    &["clean"],
    &[(0, None)],
    json(r#"[{"role": "assistant"}]"#),
)]
#[case::empty_response_is_written(
    json(r#"[{"content": "secret"}]"#),
    &[""],
    &[(0, None)],
    json(r#"[{"content": ""}]"#),
)]
fn a_response_replaces_the_text_it_was_extracted_from(
    #[case] messages: JsonValue,
    #[case] responses: &[&str],
    #[case] mappings: &[TextMapping],
    #[case] expected: JsonValue,
) {
    let (result, rewritten) = rewrite(messages, responses, mappings);
    assert_eq!(result, Ok(()));
    assert_eq!(rewritten, expected);
}

#[rstest]
#[case::response_without_mapping(
    json(r#"[{"content": "a"}]"#),
    &["x", "y"],
    &[(0, None)],
    ApplyError::UnmappedResponse { response_index: 1 },
)]
#[case::mapping_past_the_last_message(
    json(r#"[{"content": "a"}]"#),
    &["x"],
    &[(1, None)],
    ApplyError::MissingEntry { index: 1 },
)]
#[case::list_slot_past_the_last_item(
    json(r#"[{"content": [{"text": "a"}]}]"#),
    &["x"],
    &[(0, Some(1))],
    ApplyError::MissingContentItem { index: 0, content_index: 1 },
)]
#[case::list_item_that_is_not_an_object(
    json(r#"[{"content": ["a"]}]"#),
    &["x"],
    &[(0, Some(0))],
    ApplyError::MissingContentItem { index: 0, content_index: 0 },
)]
fn an_unplaceable_response_is_an_error(
    #[case] messages: JsonValue,
    #[case] responses: &[&str],
    #[case] mappings: &[TextMapping],
    #[case] expected: ApplyError,
) {
    assert_eq!(rewrite(messages, responses, mappings).0, Err(expected));
}
