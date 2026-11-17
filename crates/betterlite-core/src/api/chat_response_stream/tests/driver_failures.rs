use std::sync::Arc;
use std::sync::atomic::Ordering;

use betterlite_models::decisions::Json;

use crate::api::messages_stream_split::errors::StreamFailure;
use crate::api::response_id_codecs::response::decode_response_id;

use super::super::error::ChatResponseStreamError;
use super::fixtures::{
    Reply, Scripted, drain, drain_async, driver, driver_with_source, field, json, kinds,
};

const CONTENT_HI: &str = r#"{"id":"chat_a","created":0,"choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":null}]}"#;
const FINISH_STOP: &str =
    r#"{"id":"chat_a","created":0,"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;

#[test]
fn driver_stream_failure_sets_finished_and_stops() {
    let failure = StreamFailure::Reported {
        status_code: 401,
        message: String::from("bad key"),
    };
    let source = Scripted::new(vec![Ok(json(CONTENT_HI)), Err(failure.clone())]);
    let pulls = Arc::clone(&source.pulls);
    let (mut stream, _received) = driver_with_source(source, Reply::Text, "{}", &["u1", "u2"]);
    let mut seen: u32 = 0;
    let error = loop {
        match stream.next_event() {
            Ok(Some(_event)) => seen += 1,
            Ok(None) => panic!("the stream fails after the text"),
            Err(error) => break error,
        }
    };
    assert_eq!(seen, 5, "the text was emitted before the failure");
    assert!(matches!(
        error,
        ChatResponseStreamError::Stream(observed) if observed == failure
    ));
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

#[tokio::test]
async fn driver_async_stream_failure_sets_finished_and_stops() {
    let (mut stream, _received) = driver(
        vec![
            Ok(json(CONTENT_HI)),
            Err(StreamFailure::Reported {
                status_code: 500,
                message: String::from("boom"),
            }),
        ],
        Reply::Text,
        "{}",
        &["u1", "u2"],
    );
    let mut seen: u32 = 0;
    let error = loop {
        match stream.next_event_async().await {
            Ok(Some(_event)) => seen += 1,
            Ok(None) => panic!("the stream fails after the text"),
            Err(error) => break error,
        }
    };
    assert_eq!(seen, 5, "the text was emitted before the failure");
    assert!(matches!(error, ChatResponseStreamError::Stream(_)));
    assert!(stream.state.progress.finished);
    let pulls = stream.source.pulls.load(Ordering::SeqCst);
    assert!(
        stream
            .next_event_async()
            .await
            .expect("the failed stream stops")
            .is_none()
    );
    assert_eq!(stream.source.pulls.load(Ordering::SeqCst), pulls);
}

#[tokio::test]
async fn driver_async_skips_a_null_chunk_after_priming() {
    let (mut stream, _received) = driver(
        vec![Ok(json(CONTENT_HI)), Ok(Json::Null), Ok(json(FINISH_STOP))],
        Reply::Text,
        "{}",
        &["u1", "u2", "u3"],
    );
    let events = drain_async(&mut stream).await;
    assert_eq!(events.len(), 9);
    assert_eq!(
        kinds(&events).last().map(String::as_str),
        Some("response.completed")
    );
}

#[tokio::test]
async fn driver_async_priming_skips_a_null_chunk() {
    let (mut stream, _received) = driver(
        vec![Ok(Json::Null), Ok(json(CONTENT_HI)), Ok(json(FINISH_STOP))],
        Reply::Text,
        "{}",
        &["u1", "u2", "u3"],
    );
    let events = drain_async(&mut stream).await;
    let response = field(&events[0], "response");
    let Json::Text(id) = field(&response, "id") else {
        panic!("the created response id is text");
    };
    assert_eq!(
        decode_response_id(&id).response_id.as_deref(),
        Some("chat_a")
    );
}

#[test]
fn driver_dropping_releases_the_source() {
    let source = Scripted::from_texts(&[CONTENT_HI]);
    let released = Arc::clone(&source.released);
    let (mut stream, _received) = driver_with_source(source, Reply::Text, "{}", &["u1", "u2"]);
    stream.next_event().expect("the first event builds");
    assert!(!released.load(Ordering::SeqCst));
    drop(stream);
    assert!(
        released.load(Ordering::SeqCst),
        "dropping the driver drops the provider stream"
    );
}

#[test]
fn driver_error_after_the_primed_chunk_keeps_the_earlier_events() {
    let (mut stream, _received) = driver(
        vec![
            Ok(json(CONTENT_HI)),
            Err(StreamFailure::Unclassified {
                text: String::from("connection reset"),
            }),
        ],
        Reply::Text,
        "{}",
        &["u1", "u2"],
    );
    let mut error = None;
    loop {
        match stream.next_event() {
            Ok(Some(_event)) => {}
            Ok(None) => break,
            Err(failure) => {
                error = Some(failure);
                break;
            }
        }
    }
    let Some(ChatResponseStreamError::Stream(failure)) = error else {
        panic!("the transport failure surfaces");
    };
    assert_eq!(
        failure,
        StreamFailure::Unclassified {
            text: String::from("connection reset"),
        }
    );
    let events = drain(&mut stream);
    assert!(events.is_empty(), "later polls produce no terminal success");
}
