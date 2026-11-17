use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::api::response_id_codecs::response::decode_response_id;

use super::super::error::ChatResponseStreamError;
use super::fixtures::{
    Reply, Scripted, drain, drain_async, driver, driver_with_source, event_json, field, json,
    kinds, sequence,
};

const CONTENT_HI: &str = r#"{"id":"chat_a","created":0,"choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":null}]}"#;
const FINISH_STOP: &str =
    r#"{"id":"chat_b","created":0,"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;

fn decoded_response_id(event: &Json) -> String {
    let response = field(event, "response");
    let Json::Text(id) = field(&response, "id") else {
        panic!("the event response carries an id");
    };
    decode_response_id(&id)
        .response_id
        .expect("the response id decodes")
}

fn event_type(event: &Json) -> Json {
    field(event, "type")
}

fn text_deltas(events: &[Json]) -> Vec<Json> {
    events
        .iter()
        .filter(|event| event_type(event) == Json::Text(String::from("response.output_text.delta")))
        .map(|event| field(event, "delta"))
        .collect()
}

// The created event must wait for the primed chunk, so the first pull is asserted inside the loop
fn primed_events(source: Scripted, pulls: &Arc<AtomicUsize>) -> Vec<Json> {
    let (mut stream, _received) =
        driver_with_source(source, Reply::Text, "{}", &["u1", "u2", "u3"]);
    let mut events = Vec::new();
    while let Some(event) = stream.next_event().expect("the stream does not fail") {
        events.push(event_json(&event));
        if events.len() == 1 {
            assert_eq!(
                pulls.load(Ordering::SeqCst),
                1,
                "created waits for the first chunk"
            );
            assert_eq!(decoded_response_id(&events[0]), "chat_a");
        }
        assert!(events.len() < 32, "the stream ends");
    }
    events
}

#[test]
fn driver_primed_identity_waits_for_the_first_chunk() {
    let source = Scripted::from_texts(&[CONTENT_HI, FINISH_STOP]);
    let pulls = Arc::clone(&source.pulls);
    let events = primed_events(source, &pulls);
    assert_eq!(decoded_response_id(&events[1]), "chat_a");
    assert_eq!(
        decoded_response_id(&events[8]),
        "chat_a",
        "a later chunk id never wins"
    );
}

#[test]
fn driver_text_order_reaches_the_completed_response() {
    let source = Scripted::from_texts(&[CONTENT_HI, FINISH_STOP]);
    let pulls = Arc::clone(&source.pulls);
    let events = primed_events(source, &pulls);
    assert_eq!(
        kinds(&events),
        [
            "response.created",
            "response.in_progress",
            "response.output_item.added",
            "response.content_part.added",
            "response.output_text.delta",
            "response.output_text.done",
            "response.content_part.done",
            "response.output_item.done",
            "response.completed",
        ]
    );
    let sequences: Vec<Option<i64>> = events.iter().map(sequence).collect();
    assert_eq!(
        sequences,
        [
            Some(1),
            Some(2),
            Some(4),
            Some(5),
            Some(6),
            None,
            None,
            Some(1),
            None,
        ],
        "the ensured output item takes a sequence no event carries"
    );
    assert_eq!(text_deltas(&events), [Json::Text(String::from("hi"))]);
}

#[test]
fn driver_reads_only_the_first_choice_delta() {
    let two_choices = json(
        r#"{"id":"chat_a","choices":[{"index":0,"delta":{"content":"first"}},{"index":1,"delta":{"content":"second"}}]}"#,
    );
    let (mut stream, _received) = driver(
        vec![Ok(two_choices), Ok(json(FINISH_STOP))],
        Reply::Text,
        "{}",
        &["u1", "u2", "u3"],
    );
    let events = drain(&mut stream);
    assert_eq!(
        text_deltas(&events),
        [Json::Text(String::from("first"))],
        "only the first choice of a chunk reaches the stream"
    );
}

#[rstest]
#[case::numeric_id(r#"{"id":5,"created":0,"choices":[{"index":0,"delta":{"content":"hi"}}]}"#)]
#[case::empty_id(r#"{"id":"","created":0,"choices":[{"index":0,"delta":{"content":"hi"}}]}"#)]
fn driver_priming_uses_the_generated_id_without_a_usable_upstream_id(#[case] first: &str) {
    let (mut stream, _received) = driver(
        vec![Ok(json(first)), Ok(json(FINISH_STOP))],
        Reply::Text,
        "{}",
        &["u1", "u2", "u3"],
    );
    let events = drain(&mut stream);
    assert_eq!(decoded_response_id(&events[0]), "resp_u1");
    assert_eq!(decoded_response_id(&events[8]), "resp_u1");
}

#[test]
fn driver_priming_skips_a_null_chunk() {
    let (mut stream, _received) = driver(
        vec![Ok(Json::Null), Ok(json(CONTENT_HI)), Ok(json(FINISH_STOP))],
        Reply::Text,
        "{}",
        &["u1", "u2", "u3"],
    );
    let events = drain(&mut stream);
    assert_eq!(decoded_response_id(&events[0]), "chat_a");
    assert!(events.iter().any(|event| {
        event_type(event) == Json::Text(String::from("response.output_text.delta"))
    }));
}

#[test]
fn driver_priming_keeps_the_first_adopted_id() {
    let (mut stream, _received) = driver(
        vec![
            Ok(json(r#"{"id":"chat_a","choices":[]}"#)),
            Ok(json(r#"{"id":"chat_b","choices":[]}"#)),
        ],
        Reply::None,
        "{}",
        &["u1"],
    );
    stream
        .prime_id()
        .expect("the first chunk primes the stream");
    stream.state.progress.response_id_primed = false;
    stream
        .prime_id()
        .expect("the second chunk primes the stream again");
    assert_eq!(
        stream.state.progress.cached_response_id.as_deref(),
        Some("chat_a"),
        "a second primed chunk never replaces the adopted id"
    );
}

#[test]
fn driver_priming_latches_eof_without_another_read() {
    let source = Scripted::new(Vec::new());
    let pulls = Arc::clone(&source.pulls);
    let (mut stream, _received) = driver_with_source(source, Reply::None, "{}", &["u1"]);
    let events = drain(&mut stream);
    assert_eq!(kinds(&events), ["response.created", "response.in_progress"]);
    assert_eq!(decoded_response_id(&events[0]), "resp_u1");
    assert_eq!(
        pulls.load(Ordering::SeqCst),
        1,
        "EOF is latched after one read"
    );
    assert!(stream.state.progress.upstream_exhausted);
    assert!(stream.state.progress.finished);
}

#[tokio::test]
async fn driver_async_priming_latches_eof_without_another_read() {
    let (mut stream, _received) = driver(Vec::new(), Reply::None, "{}", &["u1"]);
    let events = drain_async(&mut stream).await;
    assert_eq!(kinds(&events), ["response.created", "response.in_progress"]);
    assert_eq!(decoded_response_id(&events[0]), "resp_u1");
    assert_eq!(
        stream.source.pulls.load(Ordering::SeqCst),
        1,
        "EOF is latched after one asynchronous read"
    );
    assert!(stream.state.progress.upstream_exhausted);
    assert!(stream.state.progress.finished);
}

#[tokio::test]
async fn driver_async_priming_adopts_the_upstream_id() {
    let (mut stream, _received) = driver(
        vec![Ok(json(CONTENT_HI)), Ok(json(FINISH_STOP))],
        Reply::Text,
        "{}",
        &["u1", "u2", "u3"],
    );
    let events = drain_async(&mut stream).await;
    assert_eq!(decoded_response_id(&events[0]), "chat_a");
    assert_eq!(
        decoded_response_id(events.last().expect("the stream ends with an event")),
        "chat_a",
        "the completed response keeps the primed identity"
    );
    assert_eq!(
        stream.source.pulls.load(Ordering::SeqCst),
        3,
        "the asynchronous end of the stream is latched too, so the terminal events stop pulling"
    );
}

#[test]
fn driver_buffered_chunk_is_taken_once() {
    let source = Scripted::from_texts(&[CONTENT_HI, FINISH_STOP]);
    let pulls = Arc::clone(&source.pulls);
    let (mut stream, received) = driver_with_source(source, Reply::Text, "{}", &["u1", "u2", "u3"]);
    let events = drain(&mut stream);
    assert_eq!(events.len(), 9);
    assert_eq!(
        pulls.load(Ordering::SeqCst),
        3,
        "one pull primes, one reads the second chunk, one finds the end"
    );
    assert_eq!(
        received.lock().expect("the calls lock").len(),
        1,
        "the end-of-stream assembly runs once"
    );
    assert!(stream.state.progress.buffered_chunk.is_none());
}

#[test]
fn driver_role_only_choice_creates_the_message_shell() {
    let (mut stream, _received) = driver(
        vec![Ok(json(
            r#"{"id":"chat_a","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}"#,
        ))],
        Reply::None,
        "{}",
        &["u1"],
    );
    let events = drain(&mut stream);
    assert_eq!(
        kinds(&events),
        [
            "response.created",
            "response.in_progress",
            "response.output_item.added",
            "response.content_part.added",
        ]
    );
    let item = field(&events[2], "item");
    assert_eq!(field(&item, "type"), Json::Text(String::from("message")));
    assert_eq!(field(&item, "role"), Json::Text(String::from("assistant")));
    assert_eq!(field(&item, "content"), Json::List(Vec::new()));
    assert_eq!(field(&events[2], "output_index"), json("0"));
}

#[test]
fn driver_choice_less_chunk_creates_no_message_shell() {
    let (mut stream, received) = driver(
        vec![Ok(json(r#"{"id":"chat_a","choices":[]}"#))],
        Reply::None,
        "{}",
        &["u1"],
    );
    let events = drain(&mut stream);
    assert_eq!(kinds(&events), ["response.created", "response.in_progress"]);
    let chunks = received.lock().expect("the calls lock").clone();
    assert_eq!(chunks.len(), 1);
    assert_eq!(
        chunks[0].len(),
        1,
        "the choice-less chunk is still snapshotted"
    );
}

#[test]
fn driver_snapshot_precedes_the_pending_events() {
    let (mut stream, received) = driver(
        vec![Ok(json(
            r#"{"id":"chat_a","choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":"content_filter"}]}"#,
        ))],
        Reply::Text,
        "{}",
        &["u1", "u2"],
    );
    let events = drain(&mut stream);
    assert!(
        events.iter().any(|event| {
            event_type(event) == Json::Text(String::from("response.output_item.added"))
        }),
        "the item added event comes from the same chunk"
    );
    let chunks = received.lock().expect("the calls lock").clone();
    let first = &chunks[0][0];
    let Some(Json::List(choices)) = first.get("choices") else {
        panic!("the snapshot keeps the choices");
    };
    let Json::Object(choice) = &choices[0] else {
        panic!("the snapshot choice is an object");
    };
    assert_eq!(
        choice.get("finish_reason"),
        Some(&Json::Text(String::from("content_filter"))),
        "the finish reason reaches the snapshot even though the chunk returned an earlier event"
    );
}

#[test]
fn driver_queued_events_come_before_the_next_chunk() {
    let source = Scripted::from_texts(&[
        CONTENT_HI,
        r#"{"id":"chat_a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"lookup","arguments":"0123456789abcdef"}}]},"finish_reason":null}]}"#,
    ]);
    let pulls = Arc::clone(&source.pulls);
    let (mut stream, _received) = driver_with_source(source, Reply::ToolCall, "{}", &["u1", "u2"]);
    let events = drain(&mut stream);
    let arguments = events
        .iter()
        .filter(|event| {
            event_type(event) == Json::Text(String::from("response.function_call_arguments.delta"))
        })
        .count();
    assert_eq!(
        arguments, 2,
        "the argument delta splits into ten-character pieces"
    );
    assert_eq!(
        pulls.load(Ordering::SeqCst),
        3,
        "neither queued text nor queued tool events read another chunk"
    );
}

#[tokio::test]
async fn driver_async_queued_events_come_before_the_next_chunk() {
    let (mut stream, _received) = driver(
        vec![
            Ok(json(CONTENT_HI)),
            Ok(json(
                r#"{"id":"chat_a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"lookup","arguments":"0123456789abcdef"}}]},"finish_reason":null}]}"#,
            )),
        ],
        Reply::ToolCall,
        "{}",
        &["u1", "u2"],
    );
    let events = drain_async(&mut stream).await;
    let arguments = events
        .iter()
        .filter(|event| {
            event_type(event) == Json::Text(String::from("response.function_call_arguments.delta"))
        })
        .count();
    assert_eq!(
        arguments, 2,
        "the argument delta splits into ten-character pieces"
    );
    assert_eq!(
        stream.source.pulls.load(Ordering::SeqCst),
        3,
        "the asynchronous end of the stream is latched, so the terminal tool events stop pulling"
    );
}

#[test]
fn driver_iterates_through_the_iterator_impl() {
    let (mut stream, _received) = driver(
        vec![Ok(json(CONTENT_HI)), Ok(json(FINISH_STOP))],
        Reply::Text,
        "{}",
        &["u1", "u2", "u3"],
    );
    let events: Vec<Json> = stream
        .by_ref()
        .map(|event| event_json(&event.expect("the stream does not fail")))
        .collect();
    assert_eq!(events.len(), 9);
    assert_eq!(
        kinds(&events).first().map(String::as_str),
        Some("response.created")
    );
}

#[test]
fn driver_sync_rejects_a_null_chunk_after_priming() {
    let source = Scripted::new(vec![
        Ok(json(CONTENT_HI)),
        Ok(Json::Null),
        Ok(json(FINISH_STOP)),
    ]);
    let pulls = Arc::clone(&source.pulls);
    let (mut stream, _received) =
        driver_with_source(source, Reply::Text, "{}", &["u1", "u2", "u3"]);
    let mut failure = None;
    loop {
        match stream.next_event() {
            Ok(Some(_event)) => {}
            Ok(None) => break,
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
    }
    let Some(error) = failure else {
        panic!("the null chunk fails the stream");
    };
    assert!(matches!(error, ChatResponseStreamError::InvalidChunk));
    assert!(stream.state.progress.finished);
    let pulls_at_failure = pulls.load(Ordering::SeqCst);
    assert!(
        stream
            .next_event()
            .expect("the failed stream stops")
            .is_none()
    );
    assert_eq!(pulls.load(Ordering::SeqCst), pulls_at_failure);
}
