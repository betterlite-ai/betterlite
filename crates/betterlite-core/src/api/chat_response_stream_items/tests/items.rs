use betterlite_models::decisions::Json;

use crate::api::chat_response_stream_items::error::StreamItemsError;
use crate::api::chat_response_stream_items::items::{
    create_content_part_added_event, queue_message_item_added_events,
};
use crate::api::chat_response_stream_tools::error::StreamToolsError;

use super::fixtures::{json, sources, state_with_details};

fn queued(
    state: &crate::api::chat_response_stream_tools::state::ChatStreamState<String>,
) -> Vec<Json> {
    state
        .progress
        .pending_response_events
        .iter()
        .map(|event| json(&serde_json::to_string(event).expect("the event serializes")))
        .collect()
}

#[test]
fn message_item_indices() {
    let mut state = state_with_details("{}", None, None);
    let sources = sources(10, &["u"]);
    queue_message_item_added_events(&mut state, &sources).expect("the message queues");
    assert_eq!(
        queued(&state),
        vec![
            json(
                r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_u","type":"message","status":"in_progress","role":"assistant","content":[]},"sequence_number":1}"#
            ),
            json(
                r#"{"type":"response.content_part.added","item_id":"msg_u","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]},"sequence_number":2}"#
            ),
        ]
    );
    assert!(state.progress.sent_message_item_added);
    assert!(state.progress.sent_content_part_added);
    assert_eq!(state.progress.cached_item_id.as_deref(), Some("msg_u"));
    assert_eq!(state.progress.message_output_index, 0);
    assert_eq!(state.progress.sequence_number, 2);
}

#[test]
fn message_item_indices_use_the_tool_index_after_reasoning() {
    let mut state = state_with_details("{}", None, None);
    state.progress.cached_reasoning_item_id = Some(String::from("rs_a"));
    state.progress.next_tool_output_index = 3;
    let sources = sources(10, &["u"]);
    queue_message_item_added_events(&mut state, &sources).expect("the message queues");
    let events = queued(&state);
    assert_eq!(
        events[0],
        json(
            r#"{"type":"response.output_item.added","output_index":3,"item":{"id":"msg_u","type":"message","status":"in_progress","role":"assistant","content":[]},"sequence_number":1}"#
        )
    );
    assert_eq!(
        events[1],
        json(
            r#"{"type":"response.content_part.added","item_id":"msg_u","output_index":3,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]},"sequence_number":2}"#
        )
    );
    assert_eq!(state.progress.message_output_index, 3);
    assert_eq!(state.progress.next_tool_output_index, 4);
}

#[test]
fn message_item_indices_reuse_the_cached_item_id() {
    let mut state = state_with_details("{}", None, None);
    let sources = sources(10, &["u", "v"]);
    queue_message_item_added_events(&mut state, &sources).expect("the first message queues");
    queue_message_item_added_events(&mut state, &sources).expect("the second message queues");
    assert_eq!(state.progress.cached_item_id.as_deref(), Some("msg_u"));
    let events = queued(&state);
    assert_eq!(events.len(), 4);
    assert_eq!(
        events[2],
        json(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_u","type":"message","status":"in_progress","role":"assistant","content":[]},"sequence_number":3}"#
        )
    );
}

#[test]
fn content_part_added_reads_the_message_output_index() {
    let mut state = state_with_details("{}", None, None);
    state.progress.cached_item_id = Some(String::from("msg_c"));
    state.progress.message_output_index = 7;
    let event = create_content_part_added_event(&mut state, &sources(10, &[]))
        .expect("the content part builds");
    assert_eq!(
        json(&serde_json::to_string(&event).expect("the event serializes")),
        json(
            r#"{"type":"response.content_part.added","item_id":"msg_c","output_index":7,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]},"sequence_number":1}"#
        )
    );
    assert_eq!(state.progress.sequence_number, 1);
}

#[test]
fn message_item_indices_report_tool_index_overflow() {
    let mut state = state_with_details("{}", None, None);
    state.progress.cached_reasoning_item_id = Some(String::from("rs_a"));
    state.progress.next_tool_output_index = i64::MAX;
    let result = queue_message_item_added_events(&mut state, &sources(10, &["u"]));
    assert!(matches!(
        result,
        Err(StreamItemsError::Stream(StreamToolsError::IndexOverflow))
    ));
    assert!(state.progress.pending_response_events.is_empty());
}

#[test]
fn message_item_indices_report_sequence_overflow() {
    let mut state = state_with_details("{}", None, None);
    state.progress.sequence_number = i64::MAX;
    let result = queue_message_item_added_events(&mut state, &sources(10, &["u"]));
    assert!(matches!(
        result,
        Err(StreamItemsError::Stream(StreamToolsError::SequenceOverflow))
    ));
    assert!(state.progress.pending_response_events.is_empty());
    assert!(state.progress.sent_message_item_added);
    assert!(state.progress.sent_content_part_added);
}
