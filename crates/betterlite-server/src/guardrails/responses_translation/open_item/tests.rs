#![cfg(test)]
use super::{Value, open_item_closing_events, open_item_state};
use serde_json::json;

#[test]
fn open_item_state_uses_last_not_done_item() {
    let events = vec![
        json!({"type":"response.output_item.added","output_index":1_i64,"item":{"id":"msg","type":"message"}}),
        json!({"type":"response.content_part.added","item_id":"msg","content_index":2_i64}),
        json!({"type":"response.content_part.added","item_id":"msg","content_index":3_i64}),
        json!({"type":"response.content_part.done","item_id":"msg","content_index":3_i64}),
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
        vec![json!({"type":"response.output_item.added","output_index":0_i64,"item":{"id":"m"}})];
    let closed = open_item_closing_events(&events);
    assert_eq!(closed[0]["item"]["content"][0]["text"], "");
    assert!(open_item_closing_events(&[]).is_empty());
}

#[test]
fn open_item_state_returns_none_for_done_item_and_empty_id() {
    let done = vec![
        json!({"type":"response.output_item.added","output_index":2_i64,"item":{"id":"m"}}),
        json!({"type":"response.output_item.done","output_index":2_i64}),
    ];
    assert!(open_item_state(&done).is_none());
    let empty_id =
        vec![json!({"type":"response.output_item.added","output_index":3_i64,"item":{"id":""}})];
    assert!(open_item_state(&empty_id).is_none());
    let missing_id = vec![
        json!({"type":"response.output_item.added","output_index":4_i64,"item":{"type":"message"}}),
    ];
    assert!(open_item_state(&missing_id).is_none());
}

#[test]
fn open_item_state_defaults_empty_type_and_role() {
    let events = vec![
        json!({"type":"response.output_item.added","output_index":0_i64,"item":{"id":"m","type":"","role":""}}),
    ];
    let item = open_item_state(&events).unwrap();
    assert_eq!(item.item_type, "message");
    assert_eq!(item.role, "assistant");
}

#[test]
fn open_item_state_reads_type_and_role() {
    let events = vec![
        json!({"type":"response.output_item.added","output_index":0_i64,"item":{"id":"m","type":"function_call","role":"user"}}),
    ];
    let item = open_item_state(&events).unwrap();
    assert_eq!(item.item_type, "function_call");
    assert_eq!(item.role, "user");
}

#[test]
fn open_item_state_closed_part_is_not_open() {
    let events = vec![
        json!({"type":"response.output_item.added","output_index":0_i64,"item":{"id":"m","type":"message"}}),
        json!({"type":"response.content_part.added","item_id":"m","content_index":0_i64}),
        json!({"type":"response.content_part.done","item_id":"m","content_index":0_i64}),
    ];
    let item = open_item_state(&events).unwrap();
    assert!(!item.part_open);
    assert_eq!(item.content_index, 0);
    assert_eq!(open_item_closing_events(&events).len(), 1);
}

#[test]
fn closing_events_mark_function_call_incomplete() {
    let events = vec![
        json!({"type":"response.output_item.added","output_index":1_i64,"item":{"type":"function_call","id":"call","arguments":"{}"}}),
    ];
    let closed = open_item_closing_events(&events);
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0]["item"]["status"], "incomplete");
    assert_eq!(closed[0]["item"]["arguments"], "{}");
}

#[test]
fn open_item_state_closes_only_latest_unfinished_item() {
    let events = [
        json!({"type":"response.output_item.added","output_index":6_i64,"item":{"id":"first"}}),
        json!({"type":"response.output_item.added","output_index":2_i64,"item":{"id":"last"}}),
        json!({"type":"response.output_item.done","output_index":3_i64}),
        json!({"type":"response.output_item.added","output_index":3_i64,"item":{"id":"already-done"}}),
    ];
    let state = open_item_state(&events).unwrap();
    assert_eq!(state.output_index, 2);
    assert_eq!(state.item_id, "last");
    assert_eq!(state.payload, json!({"id":"last"}));
}

#[rstest::rstest]
#[case::missing(json!({"type":"response.output_item.added","output_index":2_i64}))]
#[case::null(json!({"type":"response.output_item.added","output_index":2_i64,"item":null}))]
fn open_item_state_missing_latest_payload_does_not_fall_back(#[case] last: Value) {
    let events = [
        json!({"type":"response.output_item.added","output_index":1_i64,"item":{"id":"first"}}),
        last,
    ];
    assert!(open_item_state(&events).is_none());
}

#[test]
fn closing_events_keep_item_and_part_indices_and_role() {
    let events = [
        json!({"type":"response.output_item.added","output_index":4_i64,"item":{"id":"msg","role":"user"}}),
        json!({"type":"response.content_part.added","item_id":"msg","content_index":7_i64}),
        json!({"type":"response.content_part.added","item_id":"msg","content_index":2_i64}),
        json!({"type":"response.output_text.delta","item_id":"msg","delta":"one"}),
        json!({"type":"response.output_text.delta","item_id":"msg","delta":"two"}),
    ];
    assert_eq!(
        open_item_closing_events(&events),
        vec![
            json!({"type":"response.output_text.done","item_id":"msg","output_index":4_i64,"content_index":2_i64,"text":"onetwo"}),
            json!({"type":"response.content_part.done","item_id":"msg","output_index":4_i64,"content_index":2_i64,"part":{"type":"output_text","text":"onetwo","annotations":[],"logprobs":null}}),
            json!({"type":"response.output_item.done","output_index":4_i64,"item":{"type":"message","id":"msg","status":"completed","role":"user","content":[{"type":"output_text","text":"onetwo","annotations":[]}]}}),
        ]
    );
}

#[rstest::rstest]
#[case::null(json!(null))]
#[case::array(json!([]))]
#[case::string(json!("item"))]
fn closing_events_non_object_fields_are_empty(#[case] payload: Value) {
    assert!(super::incomplete_item_fields(&payload).is_empty());
}
