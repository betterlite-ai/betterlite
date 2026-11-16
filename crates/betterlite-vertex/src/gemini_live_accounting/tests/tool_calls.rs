#![cfg(test)]

use rstest::rstest;

use super::support::uuids;
use crate::gemini_live_accounting::tool_calls::{
    FunctionCallArgumentsDone, ToolCallIds, ToolCallMessage, transform_tool_call_events,
};
use crate::gemini_live_client::state::{MAX_TOOL_CALL_NAMES, ToolCallNames};

fn message(source: &str) -> ToolCallMessage {
    serde_json::from_str(source).expect("the tool call parses")
}

fn transform(
    names: &mut ToolCallNames,
    source: &str,
    ids: ToolCallIds<'_>,
) -> Vec<FunctionCallArgumentsDone> {
    transform_tool_call_events(names, &message(source), ids, &mut uuids())
}

fn stored(names: &ToolCallNames) -> Vec<(&str, &str)> {
    names
        .entries
        .iter()
        .map(|(id, name)| (id.as_str(), name.as_str()))
        .collect()
}

#[test]
fn every_call_becomes_one_done_event_with_shared_resolved_ids() {
    let mut names = ToolCallNames::default();
    let events = transform(
        &mut names,
        r#"{"functionCalls": [{"id": "a", "name": "first", "args": {"x": 1}}, {"id": "b", "name": "second", "args": {}}]}"#,
        ToolCallIds {
            response_id: Some("resp_given"),
            output_item_id: Some("item_given"),
        },
    );
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0].event_type,
        "response.function_call_arguments.done"
    );
    assert_eq!(events[0].response_id, "resp_given");
    assert_eq!(events[1].response_id, "resp_given");
    assert_eq!(events[0].item_id, "item_given_tool_0");
    assert_eq!(events[1].item_id, "item_given_tool_1");
    assert_eq!(events[0].output_index, 0);
    assert_eq!(events[1].output_index, 1);
    assert_eq!(events[1].call_id, "b");
    assert_eq!(events[1].name, "second");
}

#[test]
fn missing_response_and_item_ids_are_generated_once_before_the_events() {
    let mut names = ToolCallNames::default();
    let events = transform(
        &mut names,
        r#"{"functionCalls": [{"id": "a", "name": "first"}, {"id": "b", "name": "second"}]}"#,
        ToolCallIds::default(),
    );
    let response_id = "resp_01010101-0101-0101-0101-010101010101";
    assert_eq!(events[0].response_id, response_id);
    assert_eq!(events[1].response_id, response_id);
    assert_eq!(
        events[0].item_id,
        "item_02020202-0202-0202-0202-020202020202_tool_0"
    );
    assert_eq!(
        events[1].item_id,
        "item_02020202-0202-0202-0202-020202020202_tool_1"
    );
    assert_eq!(
        events[0].event_id,
        "event_03030303-0303-0303-0303-030303030303"
    );
    assert_eq!(
        events[1].event_id,
        "event_04040404-0404-0404-0404-040404040404"
    );
}

#[rstest]
#[case::absent(r#"{"functionCalls": [{"name": "lookup"}]}"#)]
#[case::null(r#"{"functionCalls": [{"id": null, "name": "lookup"}]}"#)]
#[case::empty(r#"{"functionCalls": [{"id": "", "name": "lookup"}]}"#)]
fn a_missing_or_empty_call_id_is_generated_from_sixteen_uuid_digits(#[case] source: &str) {
    let mut names = ToolCallNames::default();
    let events = transform(
        &mut names,
        source,
        ToolCallIds {
            response_id: Some("resp"),
            output_item_id: Some("item"),
        },
    );
    assert_eq!(events[0].call_id, "call_0101010101010101");
    assert_eq!(stored(&names), [("call_0101010101010101", "lookup")]);
}

#[test]
fn a_supplied_call_id_is_kept_and_draws_no_uuid() {
    let mut names = ToolCallNames::default();
    let events = transform(
        &mut names,
        r#"{"functionCalls": [{"id": "given", "name": "lookup"}]}"#,
        ToolCallIds {
            response_id: Some("resp"),
            output_item_id: Some("item"),
        },
    );
    assert_eq!(events[0].call_id, "given");
    assert_eq!(
        events[0].event_id,
        "event_01010101-0101-0101-0101-010101010101"
    );
}

#[rstest]
#[case::absent(r#"{"id": "a", "name": "f"}"#, "{}")]
#[case::object(
    r#"{"id": "a", "name": "f", "args": {"city": "Zürich", "n": [1, 2], "ok": true}}"#,
    r#"{"city": "Z\u00fcrich", "n": [1, 2], "ok": true}"#
)]
#[case::explicit_null(r#"{"id": "a", "name": "f", "args": null}"#, "null")]
#[case::astral(
    r#"{"id": "a", "name": "f", "args": {"k": "😀"}}"#,
    r#"{"k": "\ud83d\ude00"}"#
)]
fn arguments_are_dumped_with_python_spacing_and_ascii_escapes(
    #[case] call: &str,
    #[case] expected: &str,
) {
    let mut names = ToolCallNames::default();
    let events = transform(
        &mut names,
        &format!(r#"{{"functionCalls": [{call}]}}"#),
        ToolCallIds {
            response_id: Some("resp"),
            output_item_id: Some("item"),
        },
    );
    assert_eq!(events[0].arguments, expected);
}

#[test]
fn a_call_without_args_dumps_an_empty_object() {
    let mut names = ToolCallNames::default();
    let events = transform(
        &mut names,
        r#"{"functionCalls": [{"id": "a", "name": "f"}]}"#,
        ToolCallIds {
            response_id: Some("resp"),
            output_item_id: Some("item"),
        },
    );
    assert_eq!(events[0].arguments, "{}");
}

#[test]
fn no_function_calls_emit_no_events_and_remember_nothing() {
    let mut names = ToolCallNames::default();
    let events = transform(&mut names, "{}", ToolCallIds::default());
    assert!(events.is_empty());
    assert!(names.entries.is_empty());
}

#[test]
fn a_null_function_call_list_is_an_empty_list() {
    let mut names = ToolCallNames::default();
    let events = transform(
        &mut names,
        r#"{"functionCalls": null}"#,
        ToolCallIds::default(),
    );
    assert!(events.is_empty());
    assert!(names.entries.is_empty());
}

#[test]
fn a_null_name_takes_the_empty_default_and_is_not_remembered() {
    let mut names = ToolCallNames::default();
    let events = transform(
        &mut names,
        r#"{"functionCalls": [{"id": "a", "name": null, "args": {"k": 1}}]}"#,
        ToolCallIds {
            response_id: Some("resp"),
            output_item_id: Some("item"),
        },
    );
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].name, "");
    assert_eq!(events[0].arguments, r#"{"k": 1}"#);
    assert!(names.entries.is_empty());
}

#[test]
fn only_calls_with_an_id_and_a_name_are_remembered() {
    let mut names = ToolCallNames::default();
    let events = transform(
        &mut names,
        r#"{"functionCalls": [{"id": "a"}, {"id": "b", "name": "kept"}]}"#,
        ToolCallIds {
            response_id: Some("resp"),
            output_item_id: Some("item"),
        },
    );
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].name, "");
    assert_eq!(stored(&names), [("b", "kept")]);
}

fn call_batch(first: usize, last: usize) -> String {
    let calls: Vec<String> = (first..last)
        .map(|index| format!(r#"{{"id": "call_{index}", "name": "tool_{index}"}}"#))
        .collect();
    format!(r#"{{"functionCalls": [{}]}}"#, calls.join(", "))
}

#[test]
fn the_oldest_names_are_evicted_until_the_cap_remains() {
    let mut names = ToolCallNames::default();
    let ids = ToolCallIds {
        response_id: Some("resp"),
        output_item_id: Some("item"),
    };
    transform(&mut names, &call_batch(0, MAX_TOOL_CALL_NAMES + 3), ids);
    assert_eq!(names.entries.len(), MAX_TOOL_CALL_NAMES);
    assert!(!names.entries.contains_key("call_2"));
    assert!(names.entries.contains_key("call_3"));
    assert!(names.entries.contains_key("call_258"));
}

#[test]
fn a_retried_call_id_is_refreshed_so_the_next_eviction_spares_it() {
    let mut names = ToolCallNames::default();
    let ids = ToolCallIds {
        response_id: Some("resp"),
        output_item_id: Some("item"),
    };
    transform(&mut names, &call_batch(0, MAX_TOOL_CALL_NAMES), ids);
    names.refresh("call_0");
    transform(
        &mut names,
        r#"{"functionCalls": [{"id": "call_new", "name": "tool_new"}]}"#,
        ids,
    );
    assert!(names.entries.contains_key("call_0"));
    assert!(!names.entries.contains_key("call_1"));
    assert!(names.entries.contains_key("call_new"));
}

#[test]
fn announcing_a_known_call_id_again_replaces_its_name_and_keeps_the_cap() {
    let mut names = ToolCallNames::default();
    let ids = ToolCallIds {
        response_id: Some("resp"),
        output_item_id: Some("item"),
    };
    transform(&mut names, &call_batch(0, MAX_TOOL_CALL_NAMES), ids);
    transform(
        &mut names,
        r#"{"functionCalls": [{"id": "call_0", "name": "renamed"}]}"#,
        ids,
    );
    assert_eq!(names.entries.len(), MAX_TOOL_CALL_NAMES);
    assert_eq!(
        names.entries.get("call_0").map(String::as_str),
        Some("renamed")
    );
    assert!(names.entries.contains_key("call_1"));
}
