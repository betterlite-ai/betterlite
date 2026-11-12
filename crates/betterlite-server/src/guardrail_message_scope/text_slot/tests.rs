#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;
use rstest::rstest;

use super::{message_slot_texts, message_with_slot_texts};

fn message(source: &str) -> Object {
    let GuardrailValue::Object(fields) = serde_json::from_str(source).unwrap() else {
        panic!("not an object: {source}");
    };
    fields
}

fn texts(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

#[rstest]
#[case::string_content(r#"{"role":"user","content":"hello"}"#, vec!["hello"])]
#[case::empty_string_is_a_slot(r#"{"content":""}"#, vec![""])]
#[case::text_parts(r#"{"content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]}"#, vec!["a", "b"])]
#[case::image_part_is_not_a_slot(r#"{"content":[{"type":"text","text":"a"},{"type":"image_url","image_url":{"url":"u"}}]}"#, vec!["a"])]
#[case::any_part_with_a_string_text_is_a_slot(r#"{"content":[{"type":"input_text","text":"a"},{"text":"b"}]}"#, vec!["a", "b"])]
#[case::part_text_must_be_a_string(r#"{"content":[{"type":"text","text":5},{"type":"text","text":null},{"type":"text"}]}"#, Vec::<&str>::new())]
#[case::parts_that_are_not_objects(r#"{"content":["a",7,null]}"#, Vec::<&str>::new())]
#[case::empty_list("{\"content\":[]}", Vec::<&str>::new())]
#[case::null_content("{\"content\":null}", Vec::<&str>::new())]
#[case::no_content("{\"role\":\"assistant\"}", Vec::<&str>::new())]
#[case::object_content("{\"content\":{\"text\":\"a\"}}", Vec::<&str>::new())]
fn slots_are_the_content_strings(#[case] source: &str, #[case] expected: Vec<&str>) {
    assert_eq!(message_slot_texts(&message(source)), expected);
}

#[rstest]
#[case::string_content(
    r#"{"role":"user","content":"hello"}"#,
    &["HELLO"],
    Some(r#"{"role":"user","content":"HELLO"}"#)
)]
#[case::each_text_part_gets_its_text(
    r#"{"content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]}"#,
    &["A", "B"],
    Some(r#"{"content":[{"type":"text","text":"A"},{"type":"text","text":"B"}]}"#)
)]
#[case::other_parts_ride_along(
    r#"{"content":[{"type":"text","text":"a"},{"type":"image_url","image_url":{"url":"u"}},{"type":"text","text":"b","cache_control":{"type":"ephemeral"}}]}"#,
    &["A", "B"],
    Some(r#"{"content":[{"type":"text","text":"A"},{"type":"image_url","image_url":{"url":"u"}},{"type":"text","text":"B","cache_control":{"type":"ephemeral"}}]}"#)
)]
#[case::other_message_fields_stay(
    r#"{"role":"user","name":"n","content":"x"}"#,
    &["y"],
    Some(r#"{"role":"user","name":"n","content":"y"}"#)
)]
#[case::too_few_texts_for_a_string(r#"{"content":"a"}"#, &[], None)]
#[case::too_many_texts_for_a_string(r#"{"content":"a"}"#, &["x", "y"], None)]
#[case::too_few_texts_for_parts(r#"{"content":[{"text":"a"},{"text":"b"}]}"#, &["A"], None)]
#[case::too_many_texts_for_parts(r#"{"content":[{"text":"a"}]}"#, &["A", "B"], None)]
#[case::texts_for_a_message_without_slots(r#"{"content":[{"type":"image_url"}]}"#, &["x"], None)]
#[case::no_slots_and_no_texts_returns_the_message(
    r#"{"role":"assistant","content":null,"tool_calls":[]}"#,
    &[],
    Some(r#"{"role":"assistant","content":null,"tool_calls":[]}"#)
)]
#[case::no_content_and_no_texts(r#"{"role":"assistant"}"#, &[], Some(r#"{"role":"assistant"}"#))]
#[case::empty_list_and_no_texts(r#"{"content":[]}"#, &[], Some(r#"{"content":[]}"#))]
#[case::an_empty_text_is_a_rewrite(r#"{"content":"a"}"#, &[""], Some(r#"{"content":""}"#))]
fn texts_land_in_the_matching_slots(
    #[case] source: &str,
    #[case] new_texts: &[&str],
    #[case] expected: Option<&str>,
) {
    let rewritten = message_with_slot_texts(&message(source), &texts(new_texts));
    assert_eq!(rewritten, expected.map(message));
}

#[test]
fn the_input_message_is_not_changed() {
    let original = message(r#"{"content":[{"type":"text","text":"a"}]}"#);
    let kept = original.clone();
    drop(message_with_slot_texts(&original, &texts(&["A"])));
    assert_eq!(original, kept);
}
