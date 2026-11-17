#![cfg(test)]

use rstest::rstest;
use serde_json::value::RawValue;

use super::shared::tokens;
use crate::presidio_streams::unmask_sse::{PiiTokens, unmask_other_event};

fn unmasked(event: &str, tokens: &PiiTokens) -> String {
    let raw = RawValue::from_string(event.to_owned()).unwrap();
    unmask_other_event(raw, tokens).unwrap().get().to_owned()
}

#[test]
fn walks_every_output_item_and_content_block() {
    let event = r#"{"type":"response.completed","sequence_number":3,"response":{"id":"r1","output":[{"type":"message","content":[{"type":"output_text","text":"Hi <PERSON_1>"},{"type":"output_text","text":"<PERSON_1> again"}]},{"type":"message","content":[{"type":"output_text","text":"<PERSON_1>"}]}]}}"#;
    let expected = r#"{"type":"response.completed","sequence_number":3,"response":{"id":"r1","output":[{"type":"message","content":[{"type":"output_text","text":"Hi Alice"},{"type":"output_text","text":"Alice again"}]},{"type":"message","content":[{"type":"output_text","text":"Alice"}]}]}}"#;
    assert_eq!(unmasked(event, &tokens()), expected);
}

#[test]
fn items_without_text_and_null_content_stay_as_they_are() {
    let event = r#"{"type":"response.completed","response":{"output":[{"type":"reasoning","content":null},{"type":"function_call","arguments":"<PERSON_1>"},{"type":"message","content":[{"type":"refusal","refusal":"<PERSON_1>"},{"text":7},"<PERSON_1>",{"text":"<PERSON_1>"}]}]}}"#;
    let expected = r#"{"type":"response.completed","response":{"output":[{"type":"reasoning","content":null},{"type":"function_call","arguments":"<PERSON_1>"},{"type":"message","content":[{"type":"refusal","refusal":"<PERSON_1>"},{"text":7},"<PERSON_1>",{"text":"Alice"}]}]}}"#;
    assert_eq!(unmasked(event, &tokens()), expected);
}

#[rstest]
#[case::another_event_type(r#"{"type":"response.output_text.delta","response":{"output":[{"content":[{"text":"<PERSON_1>"}]}]}}"#)]
#[case::no_type(r#"{"response":{"output":[{"content":[{"text":"<PERSON_1>"}]}]}}"#)]
#[case::no_response(r#"{"type":"response.completed"}"#)]
#[case::response_is_not_an_object(r#"{"type":"response.completed","response":"<PERSON_1>"}"#)]
#[case::output_is_not_a_list(r#"{"type":"response.completed","response":{"output":"<PERSON_1>"}}"#)]
#[case::content_is_not_a_list(
    r#"{"type":"response.completed","response":{"output":[{"content":"<PERSON_1>"}]}}"#
)]
#[case::not_an_object(r#"["<PERSON_1>"]"#)]
fn events_it_does_not_unmask_are_returned_untouched(#[case] event: &str) {
    assert_eq!(unmasked(event, &tokens()), event);
}
