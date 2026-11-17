#![cfg(test)]

use std::sync::Arc;

use http::HeaderMap;
use indexmap::IndexMap;
use tokio_util::task::TaskTracker;
use tracing::Level;

use super::report_flush_outcome;
use crate::passthrough_stream::async_lifecycle::AsyncPassthroughStream;
use crate::passthrough_stream::sync_lifecycle::PassthroughStreamingResponse;
use crate::passthrough_stream::tests::{
    FakeFlushError, FakeLogging, ReplyFixture, capture, chunk_steps, collection, inline_executor,
    script,
};

#[test]
fn a_failed_flush_is_logged_with_the_chunk_count() {
    let (captured, _guard) = capture();
    report_flush_outcome(
        Err(FakeFlushError {
            message: "the spend flush failed".to_owned(),
        }),
        3,
    );
    let events = captured.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].level, Level::ERROR);
    assert_eq!(
        events[0].message,
        "the passthrough spend-tracking flush failed; 3 collected chunks dropped"
    );
    assert!(
        events[0]
            .fields
            .iter()
            .any(|field| field.contains("the spend flush failed"))
    );
}

#[test]
fn a_successful_flush_logs_nothing() {
    let (captured, _guard) = capture();
    let outcome: Result<(), FakeFlushError> = Ok(());
    report_flush_outcome(outcome, 1);
    assert!(captured.events().is_empty());
}

#[tokio::test]
async fn async_stream_end_survives_a_failing_flush() {
    let logging = Arc::new(FakeLogging::fixture().failing());
    let (spend, _sink) = collection();
    let tracker = TaskTracker::new();
    let mut stream = AsyncPassthroughStream {
        initialized: false,
        status_code: 0,
        headers: HeaderMap::new(),
        pending: Some(ReplyFixture::ok(script(chunk_steps(&["one"]))).into_pending()),
        source: None,
        spend,
        logging: Arc::clone(&logging),
        background_tasks: tracker.clone(),
        flush_scheduled: false,
        hidden_params: IndexMap::new(),
    };
    let (captured, _guard) = capture();
    assert_eq!(
        stream.next_chunk().await.expect("first chunk"),
        Some(b"one".to_vec())
    );
    assert_eq!(stream.next_chunk().await.expect("end"), None);
    tracker.close();
    tracker.wait().await;
    assert!(tracker.is_empty());
    assert!(captured.events().iter().any(|event| {
        event.level == Level::ERROR && event.message.contains("1 collected chunks dropped")
    }));
}

#[test]
fn sync_stream_end_survives_a_failing_flush() {
    let logging = Arc::new(FakeLogging::fixture().failing());
    let (spend, _sink) = collection();
    let executor = inline_executor();
    let response = ReplyFixture::ok(script(chunk_steps(&["one"]))).into_sync();
    let mut stream = PassthroughStreamingResponse {
        status_code: response.status_code,
        headers: response.headers,
        source: response.source,
        spend,
        logging: Arc::clone(&logging),
        executor: executor.clone(),
        flush_scheduled: false,
    };
    let (captured, _guard) = capture();
    assert_eq!(
        stream.next_relay_chunk().expect("first chunk"),
        Some(b"one".to_vec())
    );
    assert_eq!(stream.next_relay_chunk().expect("end"), None);
    executor.run_jobs();
    assert!(captured.messages().iter().any(|message| {
        message.contains("the passthrough spend-tracking flush failed; 1 collected chunks dropped")
    }));
}
