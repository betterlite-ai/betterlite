#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;
use serde_json::{Map, Value, json};

use super::error::InputProvenanceError;
use super::{
    InputProvenance, input_item_provenance, is_tool_call_item, is_tool_call_output_item,
    item_rewrite_field, last_message_role, provenance_unit_bounds, rewritten_input_item,
    tool_call_output_item_mapping,
};

fn message_map(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

fn provenance(raw_input: &[Value], expected_messages: &[Value]) -> Option<InputProvenance> {
    input_item_provenance(raw_input, expected_messages).unwrap()
}

fn call_item() -> Value {
    json!({"type": "function_call", "call_id": "c1", "name": "f", "arguments": "{}"})
}

fn call_message() -> Value {
    json!({
        "role": "assistant",
        "content": null,
        "tool_calls": [
            {"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}, "index": 0}
        ]
    })
}

#[test]
fn provenance_maps_one_plain_item_to_its_index() {
    let raw_input = [json!({"role": "user", "content": "u"})];
    let expected = [json!({"role": "user", "content": "u"})];
    assert_eq!(
        provenance(&raw_input, &expected),
        Some(InputProvenance {
            item_for_message: BTreeMap::from([(0, 0)]),
            tainted: BTreeSet::new(),
        })
    );
}

#[test]
fn provenance_maps_each_plain_item_to_its_index() {
    let raw_input = [
        json!({"role": "user", "content": "u"}),
        json!({"role": "assistant", "content": "a"}),
    ];
    let expected = [
        json!({"role": "user", "content": "u"}),
        json!({"role": "assistant", "content": "a"}),
    ];
    assert_eq!(
        provenance(&raw_input, &expected),
        Some(InputProvenance {
            item_for_message: BTreeMap::from([(0, 0), (1, 1)]),
            tainted: BTreeSet::new(),
        })
    );
}

#[test]
fn provenance_maps_a_tool_call_after_a_user_message() {
    let raw_input = [json!({"role": "user", "content": "u"}), call_item()];
    let expected = [json!({"role": "user", "content": "u"}), call_message()];
    assert_eq!(
        provenance(&raw_input, &expected),
        Some(InputProvenance {
            item_for_message: BTreeMap::from([(0, 0), (1, 1)]),
            tainted: BTreeSet::new(),
        })
    );
}

#[test]
fn provenance_maps_a_tool_call_joined_to_assistant_text_as_tainted() {
    let raw_input = [
        json!({"role": "assistant", "content": "hello"}),
        call_item(),
    ];
    let expected = [json!({
        "role": "assistant",
        "content": "hello",
        "tool_calls": [
            {"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}, "index": 0}
        ]
    })];
    assert_eq!(
        provenance(&raw_input, &expected),
        Some(InputProvenance {
            item_for_message: BTreeMap::new(),
            tainted: BTreeSet::from([0]),
        })
    );
}

#[test]
fn provenance_skips_an_item_that_produced_no_message() {
    let raw_input = [json!({"role": "user", "content": null}), call_item()];
    let expected = [call_message()];
    assert_eq!(
        provenance(&raw_input, &expected),
        Some(InputProvenance {
            item_for_message: BTreeMap::from([(0, 1)]),
            tainted: BTreeSet::new(),
        })
    );
}

#[test]
fn provenance_null_when_an_item_is_not_an_object() {
    assert_eq!(
        provenance(&[json!("not an item")], &[]),
        None,
        "a non-object item is untraceable"
    );
}

#[test]
fn provenance_null_when_the_conversion_differs_from_the_messages() {
    let raw_input = [json!({"role": "user", "content": "u"})];
    assert_eq!(
        provenance(&raw_input, &[json!({"role": "assistant", "content": "u"})]),
        None
    );
}

#[test]
fn provenance_null_when_a_unit_conversion_differs_from_the_whole() {
    let raw_input = [
        json!({"type": "web_search_call", "id": "w1", "status": "completed", "action": {"type": "search", "query": "a"}}),
        json!({"type": "web_search_call", "id": "w2", "status": "completed", "action": {"type": "search", "query": "b"}}),
    ];
    let first = concat!(
        r#"Hosted web search: {"id":"w1","action":{"type":"search","query":"a"},"#,
        r#""status":"completed","type":"web_search_call"}"#
    );
    let second = concat!(
        r#"Hosted web search: {"id":"w2","action":{"type":"search","query":"b"},"#,
        r#""status":"completed","type":"web_search_call"}"#
    );
    let expected = [json!({
        "role": "assistant",
        "content": [
            {"type": "text", "text": first},
            {"type": "text", "text": second}
        ]
    })];
    assert_eq!(
        provenance(&raw_input, &expected),
        None,
        "separate units rejoin into one message, so no single item owns it"
    );
}

#[test]
fn provenance_unit_a_tool_call_after_assistant_joins_it() {
    let raw_input = [
        json!({"role": "assistant", "content": "a"}),
        json!({"type": "function_call", "call_id": "c1"}),
    ];
    let solo_conversions = vec![
        vec![json!({"role": "assistant", "content": "a"})],
        vec![json!({"role": "assistant", "content": null})],
    ];
    assert_eq!(
        provenance_unit_bounds(&raw_input, &solo_conversions),
        vec![(0, 2)]
    );
}

#[test]
fn provenance_unit_a_tool_call_after_user_starts_a_unit() {
    let raw_input = [
        json!({"role": "user", "content": "u"}),
        json!({"type": "function_call", "call_id": "c1"}),
    ];
    let solo_conversions = vec![
        vec![json!({"role": "user", "content": "u"})],
        vec![json!({"role": "assistant", "content": null})],
    ];
    assert_eq!(
        provenance_unit_bounds(&raw_input, &solo_conversions),
        vec![(0, 1), (1, 2)]
    );
}

#[test]
fn provenance_unit_an_item_without_a_message_passes_the_role_on() {
    let raw_input = [
        json!({"role": "assistant", "content": "a"}),
        json!({"type": "message", "role": "user", "content": null}),
        json!({"type": "function_call", "call_id": "c1"}),
    ];
    let solo_conversions = vec![
        vec![json!({"role": "assistant", "content": "a"})],
        Vec::new(),
        vec![json!({"role": "assistant", "content": null})],
    ];
    assert_eq!(
        provenance_unit_bounds(&raw_input, &solo_conversions),
        vec![(0, 1), (1, 3)]
    );
}

#[test]
fn provenance_unit_a_missing_first_role_does_not_join_assistant() {
    let raw_input = [
        json!({"type": "message", "role": "user", "content": null}),
        json!({"type": "function_call", "call_id": "c1"}),
    ];
    let solo_conversions = vec![
        Vec::new(),
        vec![json!({"role": "assistant", "content": null})],
    ];
    assert_eq!(
        provenance_unit_bounds(&raw_input, &solo_conversions),
        vec![(0, 1), (1, 2)]
    );
}

#[test]
fn provenance_unit_an_empty_input_has_no_bounds() {
    assert!(provenance_unit_bounds(&[], &[]).is_empty());
}

#[test]
fn provenance_error_a_converter_failure_is_not_an_untraceable_input() {
    let error = input_item_provenance(&[json!({"role": "user", "content": true})], &[]);
    assert!(matches!(
        error,
        Err(InputProvenanceError::Conversion { .. })
    ));
}

#[rstest]
#[case::untyped_with_content(json!({"content": "x"}), Some("content"))]
#[case::untyped_without_content(json!({"role": "user"}), None)]
#[case::null_type_with_content(json!({"type": null, "content": "x"}), Some("content"))]
#[case::null_type_without_content(json!({"type": null}), None)]
#[case::function_call_output(json!({"type": "function_call_output"}), Some("output"))]
#[case::custom_tool_call_output(json!({"type": "custom_tool_call_output"}), Some("output"))]
#[case::message(json!({"type": "message"}), Some("content"))]
#[case::function_call(json!({"type": "function_call"}), None)]
#[case::reasoning(json!({"type": "reasoning"}), None)]
#[case::number_type(json!({"type": 4, "content": "x"}), None)]
fn rewrite_field_by_item_type(#[case] item: Value, #[case] field: Option<&str>) {
    assert_eq!(item_rewrite_field(&message_map(item)), field);
}

#[test]
fn rewritten_item_replaces_a_string_content() {
    let item = message_map(json!({"role": "user", "content": "old"}));
    let patched = rewritten_input_item(&item, &json!({"content": "new"}))
        .unwrap()
        .unwrap();
    assert_eq!(
        Value::Object(patched),
        json!({"role": "user", "content": "new"})
    );
}

#[test]
fn rewritten_item_replaces_a_string_output_and_keeps_the_other_fields() {
    let item =
        message_map(json!({"type": "function_call_output", "call_id": "c1", "output": "old"}));
    let patched = rewritten_item(&item, &json!({"content": "new"}));
    assert_eq!(
        patched,
        json!({"type": "function_call_output", "call_id": "c1", "output": "new"})
    );
}

#[test]
fn rewritten_item_converts_non_string_content() {
    let item = message_map(json!({
        "type": "message",
        "role": "user",
        "content": [{"type": "input_text", "text": "old"}]
    }));
    let patched = rewritten_item(&item, &json!({"role": "user", "content": "plain"}));
    assert_eq!(
        patched,
        json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "plain"}]
        })
    );
}

#[test]
fn rewritten_item_converts_a_tool_output() {
    let item = message_map(json!({
        "type": "function_call_output",
        "call_id": "c1",
        "output": [{"type": "input_text", "text": "old"}]
    }));
    let patched = rewritten_item(
        &item,
        &json!({"role": "tool", "tool_call_id": "c1", "content": "new"}),
    );
    assert_eq!(
        patched,
        json!({
            "type": "function_call_output",
            "call_id": "c1",
            "output": [{"type": "input_text", "text": "new"}]
        })
    );
}

fn rewritten_item(item: &Map<String, Value>, rewritten: &Value) -> Value {
    let patched = rewritten_input_item(item, rewritten).unwrap().unwrap();
    Value::Object(patched)
}

#[test]
fn rewritten_item_null_for_a_non_object_rewrite() {
    let item = message_map(json!({"role": "user", "content": "old"}));
    assert_eq!(rewritten_input_item(&item, &json!("new")).unwrap(), None);
}

#[test]
fn rewritten_item_null_for_an_item_without_a_rewrite_field() {
    let item = message_map(json!({"type": "function_call", "call_id": "c1"}));
    assert_eq!(
        rewritten_input_item(&item, &json!({"content": "new"})).unwrap(),
        None
    );
}

#[test]
fn rewritten_item_null_when_the_conversion_has_no_matching_field() {
    let item = message_map(json!({"type": "function_call_output", "output": ["x"]}));
    assert_eq!(
        rewritten_input_item(&item, &json!({"role": "assistant", "content": "hi"})).unwrap(),
        None
    );
}

#[test]
fn rewritten_item_null_when_the_conversion_produces_no_item() {
    let item = message_map(json!({"type": "function_call_output", "output": ["x"]}));
    assert_eq!(
        rewritten_input_item(&item, &json!({"role": "user", "content": null})).unwrap(),
        None
    );
}

#[test]
fn rewritten_item_error_for_a_row_that_is_not_a_chat_message() {
    let item = message_map(json!({"type": "function_call_output", "output": ["x"]}));
    let error = rewritten_input_item(&item, &json!({"role": 5, "content": "x"}));
    assert!(matches!(error, Err(InputProvenanceError::ChatRow { .. })));
}

#[rstest]
#[case::function_call(json!({"type": "function_call"}), true)]
#[case::custom_tool_call(json!({"type": "custom_tool_call"}), true)]
#[case::message(json!({"type": "message"}), false)]
#[case::function_call_output(json!({"type": "function_call_output"}), false)]
#[case::null_type(json!({"type": null}), false)]
#[case::missing_type(json!({"call_id": "c1"}), false)]
#[case::not_an_object(json!("function_call"), false)]
fn item_predicate_is_tool_call_item(#[case] item: Value, #[case] expected: bool) {
    assert_eq!(is_tool_call_item(&item), expected);
}

#[test]
fn item_predicate_output_mapping_returns_a_call_object() {
    let item = json!({"type": "custom_tool_call", "call_id": "c1", "name": "f"});
    assert_eq!(
        tool_call_output_item_mapping(&item),
        item.as_object(),
        "the whole item is the mapping"
    );
}

#[rstest]
#[case::message(json!({"type": "message"}))]
#[case::function_call_output(json!({"type": "function_call_output"}))]
#[case::null_type(json!({"type": null}))]
#[case::not_an_object(json!("custom_tool_call"))]
fn item_predicate_output_mapping_is_none_for_other_items(#[case] item: Value) {
    assert_eq!(tool_call_output_item_mapping(&item), None);
}

#[rstest]
#[case::function_call(json!({"type": "function_call"}), true)]
#[case::custom_tool_call(json!({"type": "custom_tool_call"}), true)]
#[case::message(json!({"type": "message"}), false)]
#[case::not_an_object(json!("function_call"), false)]
fn item_predicate_is_tool_call_output_item(#[case] item: Value, #[case] expected: bool) {
    assert_eq!(is_tool_call_output_item(&item), expected);
}

#[rstest]
#[case::last_role(
    vec![json!({"role": "user"}), json!({"role": "assistant"})],
    Some("assistant")
)]
#[case::single_role(vec![json!({"role": "user"})], Some("user"))]
#[case::non_string_role(vec![json!({"role": "user"}), json!({"role": 4})], None)]
#[case::missing_role(vec![json!({"role": "user"}), json!({"content": "x"})], None)]
#[case::not_an_object(vec![json!({"role": "user"}), json!("x")], None)]
#[case::empty(vec![], None)]
fn item_predicate_last_message_role(#[case] messages: Vec<Value>, #[case] expected: Option<&str>) {
    assert_eq!(last_message_role(&messages), expected);
}
