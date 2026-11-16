#![cfg(test)]

use std::sync::Arc;

use http::HeaderMap;
use indexmap::IndexMap;
use tokio_util::task::TaskTracker;
use tracing::Level;

use super::{run_async_flush, run_sync_flush};
use crate::passthrough_stream::async_lifecycle::AsyncPassthroughStream;
use crate::passthrough_stream::spend_collection::BoxedCollector;
use crate::passthrough_stream::sync_lifecycle::PassthroughStreamingResponse;
use crate::passthrough_stream::tests::{
    FakeLogging, ReplyFixture, capture, chunk_steps, collection, inline_executor, script,
};

fn collector_with(text: &str) -> (BoxedCollector<String>, usize) {
    let (mut spend, _state) = collection();
    spend.add(text.as_bytes());
    spend.take_flush().expect("the collected chunk flushes")
}

#[tokio::test]
async fn async_flush_failure_is_logged_and_swallowed() {
    let logging = Arc::new(FakeLogging::fixture().failing());
    let (collector, chunk_count) = collector_with("one");
    let (captured, _guard) = capture();
    run_async_flush(logging.as_ref(), collector, chunk_count).await;
    assert_eq!(chunk_count, 1);
    assert_eq!(logging.flushed().len(), 1);
    let events = captured.events();
    assert!(events.iter().any(|event| {
        event.level == Level::ERROR
            && event
                .message
                .contains("the passthrough spend-tracking flush failed; 1 collected chunks dropped")
    }));
    assert!(events.iter().any(|event| {
        event
            .fields
            .iter()
            .any(|field| field.contains("the spend flush failed"))
    }));
}

#[tokio::test]
async fn async_flush_success_records_the_collector() {
    let logging = Arc::new(FakeLogging::fixture());
    let (collector, chunk_count) = collector_with("one");
    run_async_flush(logging.as_ref(), collector, chunk_count).await;
    let flushes = logging.flushed();
    assert_eq!(flushes.len(), 1);
    assert_eq!(flushes[0].logged, "one");
}

#[test]
fn sync_flush_failure_is_logged_and_swallowed() {
    let logging = Arc::new(FakeLogging::fixture().failing());
    let (collector, chunk_count) = collector_with("one");
    let (captured, _guard) = capture();
    run_sync_flush(logging.as_ref(), collector, chunk_count);
    assert_eq!(logging.flushed().len(), 1);
    let messages = captured.messages();
    assert!(messages.iter().any(|message| {
        message.contains("the passthrough spend-tracking flush failed; 1 collected chunks dropped")
    }));
}

#[test]
fn sync_flush_success_records_the_collector() {
    let logging = Arc::new(FakeLogging::fixture());
    let (collector, chunk_count) = collector_with("one");
    run_sync_flush(logging.as_ref(), collector, chunk_count);
    assert_eq!(logging.flushed()[0].logged, "one");
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
        stream.next_chunk().expect("first chunk"),
        Some(b"one".to_vec())
    );
    assert_eq!(stream.next_chunk().expect("end"), None);
    executor.run_jobs();
    assert!(captured.messages().iter().any(|message| {
        message.contains("the passthrough spend-tracking flush failed; 1 collected chunks dropped")
    }));
}
