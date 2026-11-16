#![cfg(test)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::Notify;
use tokio_util::task::TaskTracker;

use crate::passthrough_stream::async_lifecycle::AsyncPassthroughStream;
use crate::passthrough_stream::error::PassthroughStreamError;
use crate::passthrough_stream::tests::{
    FakeLogging, Fixture, ReplyFixture, SourceLog, Step, chunk_steps, collection, failed_send,
    flagged_pending, script,
};

#[tokio::test]
async fn chunks_pass_through_and_reach_the_sink() {
    let mut fixture = Fixture::awaiting(ReplyFixture::ok(script(chunk_steps(&["one", "two"]))));
    assert_eq!(
        fixture.stream.next_chunk().await.expect("first chunk"),
        Some(b"one".to_vec())
    );
    assert_eq!(
        fixture.stream.next_chunk().await.expect("second chunk"),
        Some(b"two".to_vec())
    );
    assert!(fixture.stream.initialized);
    let chunks = fixture
        .sink
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .chunks
        .clone();
    assert_eq!(chunks, vec![b"one".to_vec(), b"two".to_vec()]);
}

#[tokio::test]
async fn iteration_awaits_the_pending_send_lazily() {
    let awaited = Arc::new(AtomicBool::new(false));
    let pending = flagged_pending(
        ReplyFixture::ok(script(chunk_steps(&["one"]))),
        Arc::clone(&awaited),
    );
    let mut fixture = Fixture::from_pending(pending, Arc::new(Mutex::new(SourceLog::default())));
    assert!(!awaited.load(Ordering::SeqCst));
    assert_eq!(
        fixture.stream.next_chunk().await.expect("first chunk"),
        Some(b"one".to_vec())
    );
    assert!(awaited.load(Ordering::SeqCst));
}

#[tokio::test]
async fn end_of_stream_flushes_and_closes_the_source() {
    let mut fixture = Fixture::awaiting(ReplyFixture::ok(script(chunk_steps(&["one"]))));
    assert_eq!(
        fixture.stream.next_chunk().await.expect("first chunk"),
        Some(b"one".to_vec())
    );
    assert_eq!(fixture.stream.next_chunk().await.expect("end"), None);
    fixture.settled().await;
    assert_eq!(fixture.stream.spend.chunk_count, 1);
    assert!(fixture.stream.flush_scheduled);
    let flushes = fixture.logging.flushed();
    assert_eq!(flushes.len(), 1);
    assert_eq!(flushes[0].logged, "one");
    assert_eq!(
        fixture
            .source
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed,
        1
    );
}

#[tokio::test]
async fn stream_error_flushes_closes_and_reports() {
    let steps = vec![Step::Chunk(b"one".to_vec()), Step::Fail];
    let mut fixture = Fixture::awaiting(ReplyFixture::ok(script(steps)));
    assert_eq!(
        fixture.stream.next_chunk().await.expect("first chunk"),
        Some(b"one".to_vec())
    );
    let error = fixture
        .stream
        .next_chunk()
        .await
        .expect_err("the source fails");
    assert_eq!(
        error.to_string(),
        "the upstream passthrough stream could not be read"
    );
    assert!(matches!(error, PassthroughStreamError::StreamRead(_)));
    fixture.settled().await;
    let flushes = fixture.logging.flushed();
    assert_eq!(flushes.len(), 1);
    assert_eq!(flushes[0].logged, "one");
    assert_eq!(
        fixture
            .source
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed,
        1
    );
}

#[tokio::test]
async fn empty_stream_does_not_flush() {
    let mut fixture = Fixture::awaiting(ReplyFixture::ok(script(Vec::new())));
    assert_eq!(fixture.stream.next_chunk().await.expect("end"), None);
    fixture.settled().await;
    assert!(fixture.logging.flushed().is_empty());
    assert!(!fixture.stream.flush_scheduled);
    assert_eq!(
        fixture
            .source
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed,
        1
    );
}

#[tokio::test]
async fn a_failing_collector_does_not_break_delivery() {
    let mut fixture = Fixture::awaiting(ReplyFixture::ok(script(chunk_steps(&["one", "two"]))));
    fixture
        .sink
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .fail_at_call = Some(1);
    assert_eq!(
        fixture.stream.next_chunk().await.expect("first chunk"),
        Some(b"one".to_vec())
    );
    assert_eq!(
        fixture.stream.next_chunk().await.expect("second chunk"),
        Some(b"two".to_vec())
    );
    assert_eq!(fixture.stream.next_chunk().await.expect("end"), None);
    fixture.settled().await;
    assert!(fixture.logging.flushed().is_empty());
    assert!(!fixture.stream.flush_scheduled);
    assert_eq!(
        fixture
            .sink
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .calls,
        1
    );
}

#[tokio::test]
async fn iterating_after_the_end_does_not_flush_twice() {
    let mut fixture = Fixture::awaiting(ReplyFixture::ok(script(chunk_steps(&["one"]))));
    assert_eq!(
        fixture.stream.next_chunk().await.expect("first chunk"),
        Some(b"one".to_vec())
    );
    assert_eq!(fixture.stream.next_chunk().await.expect("end"), None);
    fixture.settled().await;
    assert_eq!(fixture.stream.next_chunk().await.expect("end again"), None);
    fixture.settled().await;
    assert_eq!(fixture.logging.flushed().len(), 1);
    assert!(fixture.stream.flush_scheduled);
}

#[tokio::test]
async fn iteration_after_a_failed_send_reports_it() {
    let mut fixture =
        Fixture::from_pending(failed_send(), Arc::new(Mutex::new(SourceLog::default())));
    let error = fixture
        .stream
        .next_chunk()
        .await
        .expect_err("the send fails");
    assert_eq!(
        error.to_string(),
        "the passthrough request could not be sent"
    );
    assert!(matches!(error, PassthroughStreamError::Send(_)));
}

#[tokio::test]
async fn iteration_after_a_status_error_has_no_source() {
    let mut fixture = Fixture::awaiting(ReplyFixture::ok(script(Vec::new())).with_status(500));
    let error = fixture
        .stream
        .next_chunk()
        .await
        .expect_err("500 cannot stream");
    assert!(matches!(error, PassthroughStreamError::UpstreamStatus(_)));
    let error = fixture
        .stream
        .next_chunk()
        .await
        .expect_err("no source was stored");
    assert!(matches!(error, PassthroughStreamError::AlreadyAwaited));
}

#[tokio::test]
async fn the_relay_returns_before_the_flush_finishes() {
    let gate = Arc::new(Notify::new());
    let logging = Arc::new(FakeLogging::fixture().gated(Arc::clone(&gate)));
    let (spend, _sink) = collection();
    let tracker = TaskTracker::new();
    let mut stream = AsyncPassthroughStream::new(
        ReplyFixture::ok(script(chunk_steps(&["one"]))).into_pending(),
        spend,
        Arc::clone(&logging),
        tracker.clone(),
    );
    assert_eq!(
        stream.next_chunk().await.expect("first chunk"),
        Some(b"one".to_vec())
    );
    assert_eq!(stream.next_chunk().await.expect("end"), None);
    logging.started.notified().await;
    assert!(logging.flushed().is_empty());
    assert!(!tracker.is_empty());
    gate.notify_one();
    tracker.close();
    tracker.wait().await;
    assert_eq!(logging.flushed().len(), 1);
    assert!(tracker.is_empty());
}
