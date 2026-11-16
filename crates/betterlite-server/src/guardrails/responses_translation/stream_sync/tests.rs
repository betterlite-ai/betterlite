#![cfg(test)]

use std::collections::BTreeMap;

use crate::guardrails::tool_call_identity::ToolCallShape;
use serde_json::{Value, json};

use super::error::StreamSyncError;
use super::{
    deliver_ended_stream_tool_call_rewrites, spread_text_rewrite_over_stream_events,
    sync_stream_events_with_rewrites, tool_call_event_call_id, tool_call_ids_by_item_id,
};

fn delta(output_index: u32, content_index: u32, value: &str) -> Value {
    json!({
        "type": "response.output_text.delta",
        "item_id": "m1",
        "output_index": output_index,
        "content_index": content_index,
        "delta": value,
    })
}

fn done(output_index: u32, content_index: u32, value: &str) -> Value {
    json!({
        "type": "response.output_text.done",
        "item_id": "m1",
        "output_index": output_index,
        "content_index": content_index,
        "text": value,
    })
}

fn rewrites(entries: &[((usize, usize), &str)]) -> BTreeMap<(usize, usize), String> {
    entries
        .iter()
        .map(|(position, text)| (*position, (*text).to_owned()))
        .collect()
}

fn shape(name: Option<&str>, arguments: &str) -> ToolCallShape {
    ToolCallShape {
        name: name.map(str::to_owned),
        arguments: arguments.to_owned(),
    }
}

fn function_call_events() -> (Vec<Value>, Vec<Value>) {
    let earlier = vec![
        json!({"type": "response.output_item.added", "output_index": 0, "item": {"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "lookup", "arguments": ""}}),
        json!({"type": "response.function_call_arguments.delta", "item_id": "fc_1", "output_index": 0, "delta": "{\"q\""}),
        json!({"type": "response.function_call_arguments.delta", "item_id": "fc_1", "output_index": 0, "delta": ":1}"}),
        json!({"type": "response.function_call_arguments.done", "item_id": "fc_1", "output_index": 0, "arguments": "{\"q\":1}"}),
        json!({"type": "response.output_item.done", "output_index": 0, "item": {"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "lookup", "arguments": "{\"q\":1}"}}),
    ];
    let output = vec![
        json!({"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "lookup", "arguments": "{\"q\":1}"}),
    ];
    (earlier, output)
}

#[test]
fn sync_puts_the_rewrite_on_the_first_delta_and_blanks_the_rest() {
    let mut events = [delta(0, 0, "a"), delta(0, 0, "b"), delta(0, 0, "c")];
    sync_stream_events_with_rewrites(&mut events, &rewrites(&[((0, 0), "new")]));
    assert_eq!(events[0]["delta"], json!("new"));
    assert_eq!(events[1]["delta"], json!(""));
    assert_eq!(events[2]["delta"], json!(""));
}

#[test]
fn sync_counts_deltas_per_position() {
    let mut events = [delta(0, 0, "a"), delta(0, 1, "b"), delta(0, 0, "c")];
    sync_stream_events_with_rewrites(
        &mut events,
        &rewrites(&[((0, 0), "left"), ((0, 1), "right")]),
    );
    assert_eq!(events[0]["delta"], json!("left"));
    assert_eq!(events[1]["delta"], json!("right"));
    assert_eq!(events[2]["delta"], json!(""));
}

#[test]
fn sync_sets_the_output_text_done_text() {
    let mut events = [done(0, 0, "old")];
    sync_stream_events_with_rewrites(&mut events, &rewrites(&[((0, 0), "new")]));
    assert_eq!(events[0]["text"], json!("new"));
}

#[test]
fn sync_sets_the_content_part_done_part_text() {
    let mut events = [json!({
        "type": "response.content_part.done",
        "item_id": "m1",
        "output_index": 0,
        "content_index": 0,
        "part": {"type": "output_text", "text": "old"},
    })];
    sync_stream_events_with_rewrites(&mut events, &rewrites(&[((0, 0), "new")]));
    assert_eq!(events[0]["part"]["text"], json!("new"));
}

#[test]
fn sync_skips_a_content_part_done_without_an_object_part() {
    let mut events = [
        json!({"type": "response.content_part.done", "output_index": 0, "content_index": 0, "part": "bare"}),
        json!({"type": "response.content_part.done", "output_index": 0, "content_index": 0}),
    ];
    sync_stream_events_with_rewrites(&mut events, &rewrites(&[((0, 0), "new")]));
    assert_eq!(events[0]["part"], json!("bare"));
    assert_eq!(
        events[1],
        json!({"type": "response.content_part.done", "output_index": 0, "content_index": 0})
    );
}

#[test]
fn sync_syncs_the_output_item_done_content_entry() {
    let mut events = [json!({
        "type": "response.output_item.done",
        "output_index": 0,
        "item": {"type": "message", "id": "m1", "content": [{"type": "output_text", "text": "old"}]},
    })];
    sync_stream_events_with_rewrites(&mut events, &rewrites(&[((0, 0), "new")]));
    assert_eq!(events[0]["item"]["content"][0]["text"], json!("new"));
}

#[test]
fn sync_syncs_an_output_item_done_without_a_content_index() {
    let mut events = [json!({
        "type": "response.output_item.done",
        "output_index": 0,
        "item": {"content": [{"text": "old"}]},
    })];
    sync_stream_events_with_rewrites(&mut events, &rewrites(&[((0, 0), "new")]));
    assert_eq!(events[0]["item"]["content"][0]["text"], json!("new"));
}

#[test]
fn sync_skips_an_output_item_done_whose_content_is_not_a_list() {
    let mut events = [
        json!({"type": "response.output_item.done", "output_index": 0, "item": {"content": "bare"}}),
        json!({"type": "response.output_item.done", "output_index": 0, "item": {"id": "m1"}}),
    ];
    sync_stream_events_with_rewrites(&mut events, &rewrites(&[((0, 0), "new")]));
    assert_eq!(events[0]["item"], json!({"content": "bare"}));
    assert_eq!(events[1]["item"], json!({"id": "m1"}));
}

#[test]
fn sync_skips_a_content_index_outside_the_content() {
    let mut events = [json!({
        "type": "response.output_item.done",
        "output_index": 0,
        "item": {"content": [{"text": "old"}]},
    })];
    sync_stream_events_with_rewrites(&mut events, &rewrites(&[((0, 3), "new")]));
    assert_eq!(events[0]["item"]["content"][0]["text"], json!("old"));
}

#[test]
fn sync_skips_events_without_integer_indexes() {
    let mut events = [
        json!({"type": "response.output_text.delta", "output_index": "0", "content_index": 0, "delta": "old"}),
        json!({"type": "response.output_text.delta", "output_index": 0, "delta": "old"}),
        json!({"type": "response.output_text.delta", "delta": "old"}),
    ];
    sync_stream_events_with_rewrites(&mut events, &rewrites(&[((0, 0), "new")]));
    assert_eq!(events[0]["delta"], json!("old"));
    assert_eq!(events[1]["delta"], json!("old"));
    assert_eq!(events[2]["delta"], json!("old"));
}

#[test]
fn sync_skips_events_that_are_not_objects() {
    let mut events = [json!("bare string"), json!([1, 2])];
    let original = events.clone();
    sync_stream_events_with_rewrites(&mut events, &rewrites(&[((0, 0), "new")]));
    assert_eq!(events, original);
}

#[test]
fn sync_leaves_a_position_without_a_rewrite_alone() {
    let mut events = [delta(0, 0, "old"), done(0, 0, "old")];
    sync_stream_events_with_rewrites(&mut events, &rewrites(&[((1, 0), "new")]));
    assert_eq!(events[0]["delta"], json!("old"));
    assert_eq!(events[1]["text"], json!("old"));
}

#[test]
fn spread_puts_the_text_on_the_first_position_and_blanks_the_rest() {
    let mut events = [
        delta(0, 0, "a"),
        delta(0, 1, "b"),
        delta(0, 0, "c"),
        done(0, 0, "abc"),
    ];
    spread_text_rewrite_over_stream_events(&mut events, "new", "guard").unwrap();
    assert_eq!(events[0]["delta"], json!("new"));
    assert_eq!(events[1]["delta"], json!(""));
    assert_eq!(events[2]["delta"], json!(""));
    assert_eq!(events[3]["text"], json!("new"));
}

#[test]
fn spread_uses_the_order_of_first_appearance() {
    let mut events = [delta(1, 0, "a"), delta(0, 0, "b"), delta(1, 0, "c")];
    spread_text_rewrite_over_stream_events(&mut events, "new", "guard").unwrap();
    assert_eq!(events[0]["delta"], json!("new"));
    assert_eq!(events[1]["delta"], json!(""));
    assert_eq!(events[2]["delta"], json!(""));
}

#[test]
fn spread_skips_events_that_carry_no_text() {
    let mut events = [
        json!("bare string"),
        json!({"type": "response.output_item.added", "output_index": 0}),
        delta(0, 0, "a"),
    ];
    spread_text_rewrite_over_stream_events(&mut events, "new", "guard").unwrap();
    assert_eq!(events[2]["delta"], json!("new"));
}

#[test]
fn spread_rejects_a_scanned_event_that_is_not_an_output_text_event() {
    let mut events = [
        delta(0, 0, "a"),
        json!({"type": "response.reasoning_summary_text.delta", "output_index": 0, "summary_index": 0, "delta": "why"}),
    ];
    let error = spread_text_rewrite_over_stream_events(&mut events, "new", "guard").unwrap_err();
    assert!(matches!(error, StreamSyncError::Undeliverable { .. }));
    assert_eq!(events[0]["delta"], json!("a"));
}

#[test]
fn spread_rejects_a_scanned_text_event_without_an_integer_index() {
    let mut events = [
        delta(0, 0, "a"),
        json!({"type": "response.output_text.delta", "item_id": "m2", "output_index": 0, "delta": "b"}),
    ];
    let original = events.clone();
    let error = spread_text_rewrite_over_stream_events(&mut events, "new", "named").unwrap_err();
    assert_eq!(
        error.to_string(),
        "Guardrail 'named' rewrote the streamed response but the rewrite cannot be written back to the stream: the scanned text events are not all output_text deltas with an integer output_index and content_index, so the text rewrite has nowhere to land"
    );
    assert_eq!(events, original);
}

#[test]
fn ids_maps_an_item_id_to_its_call_id() {
    let events = [
        json!({"type": "response.output_item.added", "output_index": 0, "item": {"type": "function_call", "id": "fc_1", "call_id": "call_1"}}),
        json!({"type": "response.output_item.done", "output_index": 1, "item": {"type": "custom_tool_call", "id": "ctc_1", "call_id": "call_2"}}),
    ];
    let ids = tool_call_ids_by_item_id(&events);
    assert_eq!(ids.get("fc_1").map(String::as_str), Some("call_1"));
    assert_eq!(ids.get("ctc_1").map(String::as_str), Some("call_2"));
}

#[test]
fn ids_lets_a_later_output_item_event_overwrite_an_item_id() {
    let events = [
        json!({"type": "response.output_item.added", "output_index": 0, "item": {"type": "function_call", "id": "fc_1", "call_id": "first"}}),
        json!({"type": "response.output_item.done", "output_index": 0, "item": {"type": "function_call", "id": "fc_1", "call_id": "second"}}),
    ];
    let ids = tool_call_ids_by_item_id(&events);
    assert_eq!(ids.get("fc_1").map(String::as_str), Some("second"));
}

#[test]
fn ids_skips_items_without_string_identifiers() {
    let events = [
        json!({"type": "response.output_item.added", "item": {"type": "function_call", "id": "fc_1"}}),
        json!({"type": "response.output_item.added", "item": {"type": "function_call", "id": 7, "call_id": "call_2"}}),
        json!({"type": "response.output_item.added", "item": {"type": "message", "id": "m1", "call_id": "call_3"}}),
        json!({"type": "response.output_text.delta", "item": {"type": "function_call", "id": "fc_4", "call_id": "call_4"}}),
    ];
    assert!(tool_call_ids_by_item_id(&events).is_empty());
}

#[test]
fn ids_resolves_a_payload_event_through_its_item_id() {
    let mut ids = BTreeMap::new();
    ids.insert("fc_1".to_owned(), "call_1".to_owned());
    let events = [
        json!({"type": "response.function_call_arguments.delta", "item_id": "fc_1", "delta": "{}"}),
        json!({"type": "response.custom_tool_call_input.done", "item_id": "fc_1", "input": "raw"}),
    ];
    assert_eq!(
        tool_call_event_call_id(&events[0], &ids),
        Some("call_1".to_owned())
    );
    assert_eq!(
        tool_call_event_call_id(&events[1], &ids),
        Some("call_1".to_owned())
    );
}

#[test]
fn ids_resolves_an_output_item_event_through_its_item_call_id() {
    let ids = BTreeMap::new();
    let events = [
        json!({"type": "response.output_item.added", "item": {"type": "custom_tool_call", "call_id": "call_2"}}),
        json!({"type": "response.output_item.done", "item": {"type": "function_call", "call_id": "call_3"}}),
    ];
    assert_eq!(
        tool_call_event_call_id(&events[0], &ids),
        Some("call_2".to_owned())
    );
    assert_eq!(
        tool_call_event_call_id(&events[1], &ids),
        Some("call_3".to_owned())
    );
}

#[test]
fn ids_gives_none_for_an_event_with_neither_a_known_item_nor_a_tool_call_item() {
    let mut ids = BTreeMap::new();
    ids.insert("fc_1".to_owned(), "call_1".to_owned());
    assert!(
        tool_call_event_call_id(
            &json!({"type": "response.function_call_arguments.delta", "item_id": "fc_9", "delta": "{}"}),
            &ids
        )
        .is_none()
    );
    assert!(
        tool_call_event_call_id(
            &json!({"type": "response.function_call_arguments.delta", "delta": "{}"}),
            &ids
        )
        .is_none()
    );
    assert!(
        tool_call_event_call_id(
            &json!({"type": "response.output_item.added", "item": {"type": "message", "call_id": "call_2"}}),
            &ids
        )
        .is_none()
    );
    assert!(
        tool_call_event_call_id(
            &json!({"type": "response.output_text.delta", "call_id": "call_3"}),
            &ids
        )
        .is_none()
    );
    assert!(tool_call_event_call_id(&json!("bare string"), &ids).is_none());
}

#[test]
fn deliver_writes_a_function_call_rewrite_into_the_envelope_and_the_events() {
    let (mut earlier, mut output) = function_call_events();
    let pre = [shape(Some("lookup"), "{\"q\":1}")];
    let post = [shape(Some("search"), "{\"q\":2}")];
    deliver_ended_stream_tool_call_rewrites(&mut earlier, &mut output, &pre, &post, "guard")
        .unwrap();
    assert_eq!(output[0]["name"], json!("search"));
    assert_eq!(output[0]["arguments"], json!("{\"q\":2}"));
    assert_eq!(earlier[0]["item"]["name"], json!("search"));
    assert_eq!(earlier[0]["item"]["arguments"], json!(""));
    assert_eq!(earlier[1]["delta"], json!("{\"q\":2}"));
    assert_eq!(earlier[2]["delta"], json!(""));
    assert_eq!(earlier[3]["arguments"], json!("{\"q\":2}"));
    assert_eq!(earlier[4]["item"]["name"], json!("search"));
    assert_eq!(earlier[4]["item"]["arguments"], json!("{\"q\":2}"));
}

#[test]
fn deliver_writes_a_custom_tool_call_input_rewrite() {
    let mut earlier = vec![
        json!({"type": "response.output_item.added", "output_index": 0, "item": {"type": "custom_tool_call", "id": "ctc_1", "call_id": "call_2", "name": "grammar", "input": ""}}),
        json!({"type": "response.custom_tool_call_input.delta", "item_id": "ctc_1", "output_index": 0, "delta": "raw"}),
        json!({"type": "response.custom_tool_call_input.done", "item_id": "ctc_1", "output_index": 0, "input": "raw"}),
    ];
    let mut output = vec![
        json!({"type": "custom_tool_call", "id": "ctc_1", "call_id": "call_2", "name": "grammar", "input": "raw"}),
    ];
    let pre = [shape(Some("grammar"), "raw")];
    let post = [shape(Some("grammar"), "fixed")];
    deliver_ended_stream_tool_call_rewrites(&mut earlier, &mut output, &pre, &post, "guard")
        .unwrap();
    assert_eq!(output[0]["name"], json!("grammar"));
    assert_eq!(output[0]["input"], json!("fixed"));
    assert_eq!(earlier[0]["item"]["name"], json!("grammar"));
    assert_eq!(earlier[0]["item"]["input"], json!(""));
    assert_eq!(earlier[1]["delta"], json!("fixed"));
    assert_eq!(earlier[2]["input"], json!("fixed"));
}

#[test]
fn deliver_writes_only_the_name_when_the_arguments_did_not_change() {
    let (mut earlier, mut output) = function_call_events();
    let pre = [shape(Some("lookup"), "{\"q\":1}")];
    let post = [shape(Some("search"), "{\"q\":1}")];
    deliver_ended_stream_tool_call_rewrites(&mut earlier, &mut output, &pre, &post, "guard")
        .unwrap();
    assert_eq!(output[0]["name"], json!("search"));
    assert_eq!(output[0]["arguments"], json!("{\"q\":1}"));
    assert_eq!(earlier[1]["delta"], json!("{\"q\":1}"));
    assert_eq!(earlier[2]["delta"], json!(""));
}

#[test]
fn deliver_does_nothing_when_the_shapes_match() {
    let (mut earlier, mut output) = function_call_events();
    let original_earlier = earlier.clone();
    let original_output = output.clone();
    let pre = [shape(Some("lookup"), "{\"q\":1}")];
    deliver_ended_stream_tool_call_rewrites(&mut earlier, &mut output, &pre, &pre, "guard")
        .unwrap();
    assert_eq!(earlier, original_earlier);
    assert_eq!(output, original_output);
}

#[test]
fn deliver_reports_a_tool_call_item_without_a_call_id() {
    let mut earlier = vec![
        json!({"type": "response.output_item.added", "output_index": 0, "item": {"type": "function_call", "id": "fc_1", "name": "lookup", "arguments": ""}}),
    ];
    let mut output =
        vec![json!({"type": "function_call", "id": "fc_1", "name": "lookup", "arguments": "{}"})];
    let pre = [shape(Some("lookup"), "{}")];
    let post = [shape(Some("search"), "{}")];
    let error =
        deliver_ended_stream_tool_call_rewrites(&mut earlier, &mut output, &pre, &post, "guard")
            .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Guardrail 'guard' rewrote the streamed response but the rewrite cannot be written back to the stream: 1 of the stream's 1 tool call items carry no call_id"
    );
}

#[test]
fn deliver_reports_a_repeated_call_id() {
    let mut earlier = vec![
        json!({"type": "response.output_item.added", "output_index": 0, "item": {"type": "function_call", "id": "fc_1", "call_id": "call_1"}}),
        json!({"type": "response.output_item.added", "output_index": 1, "item": {"type": "function_call", "id": "fc_2", "call_id": "call_1"}}),
    ];
    let mut output = vec![
        json!({"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "lookup", "arguments": ""}),
        json!({"type": "function_call", "id": "fc_2", "call_id": "call_1", "name": "lookup", "arguments": ""}),
    ];
    let pre = [shape(Some("lookup"), ""), shape(Some("lookup"), "")];
    let post = [shape(Some("search"), ""), shape(Some("search"), "")];
    let error =
        deliver_ended_stream_tool_call_rewrites(&mut earlier, &mut output, &pre, &post, "guard")
            .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Guardrail 'guard' rewrote the streamed response but the rewrite cannot be written back to the stream: the stream's tool call items repeat a call_id"
    );
}

#[test]
fn deliver_reports_a_returned_tool_call_count_mismatch() {
    let (mut earlier, mut output) = function_call_events();
    let pre = [shape(Some("lookup"), "{\"q\":1}")];
    let post = [
        shape(Some("search"), "{\"q\":2}"),
        shape(Some("other"), "{}"),
    ];
    let error =
        deliver_ended_stream_tool_call_rewrites(&mut earlier, &mut output, &pre, &post, "guard")
            .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Guardrail 'guard' rewrote the streamed response but the rewrite cannot be written back to the stream: the guardrail returned 2 tool calls for the stream's 1 tool call items"
    );
}

#[test]
fn deliver_reports_an_argument_event_with_an_unknown_item_id() {
    let mut earlier = vec![
        json!({"type": "response.function_call_arguments.delta", "item_id": "fc_9", "delta": "{}"}),
    ];
    let mut output = vec![
        json!({"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "lookup", "arguments": "{}"}),
    ];
    let pre = [shape(Some("lookup"), "{}")];
    let post = [shape(Some("search"), "{}")];
    let error =
        deliver_ended_stream_tool_call_rewrites(&mut earlier, &mut output, &pre, &post, "guard")
            .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Guardrail 'guard' rewrote the streamed response but the rewrite cannot be written back to the stream: a tool call argument event names an item_id that no output_item event introduced"
    );
}

#[test]
fn deliver_reports_a_rewritten_call_with_no_event_and_writes_nothing() {
    let mut earlier = vec![
        json!({"type": "response.output_text.delta", "output_index": 0, "content_index": 0, "delta": "a"}),
    ];
    let mut output = vec![
        json!({"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "lookup", "arguments": "{}"}),
    ];
    let original_earlier = earlier.clone();
    let original_output = output.clone();
    let pre = [shape(Some("lookup"), "{}")];
    let post = [shape(Some("search"), "{}")];
    let error =
        deliver_ended_stream_tool_call_rewrites(&mut earlier, &mut output, &pre, &post, "guard")
            .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Guardrail 'guard' rewrote the streamed response but the rewrite cannot be written back to the stream: no stream event carries the rewritten call_id call_1"
    );
    assert_eq!(earlier, original_earlier);
    assert_eq!(output, original_output);
}
