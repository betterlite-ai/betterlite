use betterlite_openai::api::responses_event_records::event_types::Extra;
use betterlite_openai::api::responses_event_records::records::KnownResponseEvent;

use crate::api::chat_response_stream_items::error::StreamItemsError;
use crate::api::chat_response_stream_tools::events::output_item_event;
use crate::api::chat_response_stream_tools::state::QueuedToolEvent;

use super::super::error::ChatResponseStreamError;
use super::super::translate;
use super::fixtures::{Reply, driver, json};

const IDLE: &str = r#"{"id":"chat_a","choices":[{"index":0,"delta":{},"finish_reason":null}]}"#;
const TOOL_CALL: &str = r#"{"id":"chat_a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_9","type":"function","function":{"name":"lookup","arguments":"{}"}}]},"finish_reason":null}]}"#;

fn queued(output_index: i64, sequence: i64) -> QueuedToolEvent {
    QueuedToolEvent {
        output_index,
        event: output_item_event(output_index, Extra::new(), sequence, true),
    }
}

#[test]
fn translate_prefers_queued_annotations_over_older_tool_events() {
    let (mut stream, _received) = driver(Vec::new(), Reply::None, "{}", &["u1"]);
    stream.state.progress.pending_tool_events.push(queued(3, 7));
    stream
        .annotations
        .push_back(output_item_event(5, Extra::new(), 8, true));
    let chunk = json(IDLE);
    let chosen = translate::transform(
        &mut stream.state,
        &stream.sources,
        &chunk,
        &mut stream.annotations,
    )
    .expect("the chunk transforms")
    .expect("an event is chosen");
    assert!(matches!(
        chosen,
        KnownResponseEvent::OutputItemDone(event) if event.output_index == 5
    ));
    assert_eq!(
        stream.state.progress.pending_tool_events.len(),
        1,
        "the older tool event stays queued for a later chunk"
    );
}

#[test]
fn translate_prefers_tool_deltas_over_queued_annotations() {
    let (mut stream, _received) = driver(Vec::new(), Reply::None, "{}", &["u1"]);
    stream
        .annotations
        .push_back(output_item_event(5, Extra::new(), 8, true));
    let chunk = json(TOOL_CALL);
    let chosen = translate::transform(
        &mut stream.state,
        &stream.sources,
        &chunk,
        &mut stream.annotations,
    )
    .expect("the chunk transforms")
    .expect("an event is chosen");
    assert!(matches!(chosen, KnownResponseEvent::OutputItemAdded(_)));
    assert_eq!(stream.annotations.len(), 1, "the annotation waits");
}

#[test]
fn translate_returns_older_tool_events_when_nothing_else_is_pending() {
    let (mut stream, _received) = driver(Vec::new(), Reply::None, "{}", &["u1"]);
    stream.state.progress.pending_tool_events.push(queued(3, 7));
    let chunk = json(IDLE);
    let chosen = translate::transform(
        &mut stream.state,
        &stream.sources,
        &chunk,
        &mut stream.annotations,
    )
    .expect("the chunk transforms")
    .expect("an event is chosen");
    assert!(matches!(
        chosen,
        KnownResponseEvent::OutputItemDone(event) if event.output_index == 3
    ));
    assert!(stream.state.progress.pending_tool_events.is_empty());
}

#[test]
fn driver_pending_response_events_come_before_queued_tool_events() {
    let (mut stream, _received) = driver(Vec::new(), Reply::None, "{}", &["u1"]);
    stream.state.progress.pending_tool_events.push(queued(3, 7));
    stream
        .state
        .progress
        .pending_response_events
        .push(output_item_event(5, Extra::new(), 8, true));
    stream
        .next_event()
        .expect("the created event builds")
        .expect("the stream emits the created event");
    stream
        .next_event()
        .expect("the in-progress event builds")
        .expect("the stream emits the in-progress event");
    let response = stream
        .next_event()
        .expect("the response event builds")
        .expect("the queued response event comes first");
    assert!(matches!(
        response,
        KnownResponseEvent::OutputItemDone(event) if event.output_index == 5
    ));
    let tool = stream
        .next_event()
        .expect("the tool event builds")
        .expect("the queued tool event follows");
    assert!(matches!(
        tool,
        KnownResponseEvent::OutputItemDone(event) if event.output_index == 3
    ));
}

#[test]
fn ensure_output_item_runs_once() {
    let (mut stream, _received) = driver(Vec::new(), Reply::None, "{}", &["u1"]);
    let chunk = json(r#"{"id":"chat_a","choices":[{"index":0,"delta":{"content":"hi"}}]}"#);
    translate::ensure_output_item_for_chunk(&mut stream.state, &stream.sources, &chunk)
        .expect("the item is announced");
    let sequence = stream.state.progress.sequence_number;
    translate::ensure_output_item_for_chunk(&mut stream.state, &stream.sources, &chunk)
        .expect("the second call is a no-op");
    assert_eq!(stream.state.progress.sequence_number, sequence);
    assert_eq!(stream.state.progress.pending_response_events.len(), 2);
}

#[test]
fn close_reasoning_segment_waits_for_an_active_segment() {
    let (mut stream, _received) = driver(Vec::new(), Reply::None, "{}", &["r1"]);
    let chunk =
        json(r#"{"id":"chat_a","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#);
    translate::close_reasoning_segment(&mut stream.state, &stream.sources, &chunk)
        .expect("the idle chunk closes nothing");
    assert!(stream.state.progress.pending_response_events.is_empty());
    assert!(!stream.state.progress.reasoning_active);
}

#[test]
fn close_reasoning_segment_runs_once() {
    let (mut stream, _received) = driver(Vec::new(), Reply::None, "{}", &["r1"]);
    let reasoning =
        json(r#"{"id":"chat_a","choices":[{"index":0,"delta":{"reasoning_content":"a"}}]}"#);
    translate::ensure_output_item_for_chunk(&mut stream.state, &stream.sources, &reasoning)
        .expect("the reasoning item is announced");
    let text = json(r#"{"id":"chat_a","choices":[{"index":0,"delta":{"content":"hi"}}]}"#);
    translate::close_reasoning_segment(&mut stream.state, &stream.sources, &text)
        .expect("the text closes the segment");
    assert_eq!(
        stream.state.progress.pending_response_events.len(),
        4,
        "the reasoning item added event and the three done events wait"
    );
    translate::close_reasoning_segment(&mut stream.state, &stream.sources, &text)
        .expect("the closed segment stays closed");
    assert_eq!(
        stream.state.progress.pending_response_events.len(),
        4,
        "the segment closes once"
    );
}

#[test]
fn content_that_is_not_text_fails() {
    let (mut stream, _received) = driver(Vec::new(), Reply::None, "{}", &["u1"]);
    let truthy = json(r#"{"id":"chat_a","choices":[{"index":0,"delta":{"content":5}}]}"#);
    let failure = translate::transform(
        &mut stream.state,
        &stream.sources,
        &truthy,
        &mut stream.annotations,
    )
    .expect_err("a number is not text");
    assert!(matches!(
        failure,
        ChatResponseStreamError::FieldNotText { field: "content" }
    ));
    let falsy = json(r#"{"id":"chat_a","choices":[{"index":0,"delta":{"content":0}}]}"#);
    let chosen = translate::transform(
        &mut stream.state,
        &stream.sources,
        &falsy,
        &mut stream.annotations,
    )
    .expect("zero reads as no content");
    assert!(chosen.is_none());
}

#[test]
fn reasoning_that_is_not_text_fails() {
    let (mut stream, _received) = driver(Vec::new(), Reply::None, "{}", &["u1"]);
    let truthy = json(r#"{"id":"chat_a","choices":[{"index":0,"delta":{"reasoning_content":5}}]}"#);
    let failure = translate::transform(
        &mut stream.state,
        &stream.sources,
        &truthy,
        &mut stream.annotations,
    )
    .expect_err("a number is not reasoning text");
    assert!(matches!(
        failure,
        ChatResponseStreamError::FieldNotText {
            field: "reasoning_content"
        }
    ));
}

#[test]
fn annotations_that_are_not_a_list_fail() {
    let (mut stream, _received) = driver(Vec::new(), Reply::None, "{}", &["u1"]);
    let truthy = json(r#"{"id":"chat_a","choices":[{"index":0,"delta":{"annotations":5}}]}"#);
    let failure = translate::transform(
        &mut stream.state,
        &stream.sources,
        &truthy,
        &mut stream.annotations,
    )
    .expect_err("a number is not an annotation list");
    assert!(matches!(
        failure,
        ChatResponseStreamError::Items(StreamItemsError::AnnotationsNotList)
    ));
    let falsy = json(r#"{"id":"chat_a","choices":[{"index":0,"delta":{"annotations":{}}}]}"#);
    let chosen = translate::transform(
        &mut stream.state,
        &stream.sources,
        &falsy,
        &mut stream.annotations,
    )
    .expect("an empty object reads as no annotations");
    assert!(chosen.is_none());
    assert!(!stream.state.progress.sent_annotation_events);
}
