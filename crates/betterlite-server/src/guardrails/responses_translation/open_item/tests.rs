#![cfg(test)]
use super::*;
use serde_json::json;

#[test]
fn open_item_state_uses_last_not_done_item() {
    let events = vec![
        json!({"type":"response.output_item.added","output_index":1,"item":{"id":"msg","type":"message"}}),
        json!({"type":"response.content_part.added","item_id":"msg","content_index":2}),
        json!({"type":"response.content_part.added","item_id":"msg","content_index":3}),
        json!({"type":"response.content_part.done","item_id":"msg","content_index":3}),
        json!({"type":"response.output_text.delta","item_id":"other","delta":"ignored"}),
        json!({"type":"response.output_text.delta","item_id":"msg","delta":"hello"}),
    ];
    let item = open_item_state(&events).unwrap();
    assert_eq!(item.text, "hello");
    assert_eq!(item.content_index, 2);
    assert!(item.part_open);
    assert_eq!(item.item_type, "message");
    assert_eq!(item.role, "assistant");
}

#[test]
fn closing_events_close_message_and_missing_item_does_nothing() {
    let events =
        vec![json!({"type":"response.output_item.added","output_index":0,"item":{"id":"m"}})];
    let closed = open_item_closing_events(&events);
    assert_eq!(closed[0]["item"]["content"][0]["text"], "");
    assert!(open_item_closing_events(&[]).is_empty());
}

#[test]
fn open_item_state_returns_none_for_done_item_and_empty_id() {
    let done = vec![
        json!({"type":"response.output_item.added","output_index":2,"item":{"id":"m"}}),
        json!({"type":"response.output_item.done","output_index":2}),
    ];
    assert!(open_item_state(&done).is_none());
    let empty_id =
        vec![json!({"type":"response.output_item.added","output_index":3,"item":{"id":""}})];
    assert!(open_item_state(&empty_id).is_none());
    let missing_id = vec![
        json!({"type":"response.output_item.added","output_index":4,"item":{"type":"message"}}),
    ];
    assert!(open_item_state(&missing_id).is_none());
}

#[test]
fn open_item_state_defaults_empty_type_and_role() {
    let events = vec![
        json!({"type":"response.output_item.added","output_index":0,"item":{"id":"m","type":"","role":""}}),
    ];
    let item = open_item_state(&events).unwrap();
    assert_eq!(item.item_type, "message");
    assert_eq!(item.role, "assistant");
}

#[test]
fn closing_events_mark_function_call_incomplete() {
    let events = vec![
        json!({"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"call","arguments":"{}"}}),
    ];
    let closed = open_item_closing_events(&events);
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0]["item"]["status"], "incomplete");
    assert_eq!(closed[0]["item"]["arguments"], "{}");
}
