#![cfg(test)]
use super::*;
use serde_json::json;

#[test]
fn open_item_state_uses_last_not_done_item() {
    let events = vec![
        json!({"type":"response.output_item.added","output_index":1,"item":{"id":"msg","type":"message"}}),
        json!({"type":"response.content_part.added","item_id":"msg","content_index":2}),
        json!({"type":"response.output_text.delta","item_id":"msg","delta":"hello"}),
    ];
    let item = open_item_state(&events).unwrap();
    assert_eq!(item.text, "hello");
    assert_eq!(item.content_index, 2);
    assert!(item.part_open);
}

#[test]
fn closing_events_close_message_and_missing_item_does_nothing() {
    let events =
        vec![json!({"type":"response.output_item.added","output_index":0,"item":{"id":"m"}})];
    let closed = open_item_closing_events(&events);
    assert_eq!(closed[0]["item"]["content"][0]["text"], "");
    assert!(open_item_closing_events(&[]).is_empty());
}
