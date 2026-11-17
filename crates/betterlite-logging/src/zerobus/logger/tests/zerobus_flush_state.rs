use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::time::{sleep, timeout};
use tokio_util::task::TaskTracker;

use crate::callback_logger::CallbackLogger as _;
use crate::zerobus::client::types::ZerobusIngestFailure;
use crate::zerobus::logger::delivery::RowIngest;

use super::support::{
    FakeIngest, as_row_ingest, event, expected_ids, logger, params, push, queued_ids, settle, until,
};

#[tokio::test]
async fn zerobus_flush_state_sets_and_clears_the_flag() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let gate = ingest.block();
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let logger = logger(params(1, 3600, false), client);
    logger
        .async_log_success_event(event("a"))
        .await
        .expect("the event is accepted");
    assert!(
        until(|| ingest.inserted() == 1).await,
        "the delivery starts"
    );
    assert!(logger.inner.flushing.load(Ordering::SeqCst));
    ingest.unblock();
    gate.add_permits(1);
    settle(&logger).await;
    assert!(!logger.inner.flushing.load(Ordering::SeqCst));
    assert!(queued_ids(&logger).is_empty());
}

#[tokio::test]
async fn zerobus_flush_state_clears_the_flag_after_a_failed_flush() {
    let ingest = FakeIngest::failing(ZerobusIngestFailure {
        detail: "503".to_owned(),
        retryable: true,
    });
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let logger = logger(params(1, 3600, false), client);
    logger
        .async_log_success_event(event("a"))
        .await
        .expect("the event is accepted");
    settle(&logger).await;
    assert!(!logger.inner.flushing.load(Ordering::SeqCst));
    assert_eq!(queued_ids(&logger), expected_ids(&["a"]));
}

#[tokio::test]
async fn zerobus_flush_state_skips_a_second_flush_while_one_runs() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let gate = ingest.block();
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let logger = logger(params(1, 3600, false), client);
    logger
        .async_log_success_event(event("a"))
        .await
        .expect("the event is accepted");
    assert!(
        until(|| ingest.inserted() == 1).await,
        "the delivery starts"
    );
    let inner = Arc::clone(&logger.inner);
    timeout(Duration::from_millis(100), inner.flush_queue(true))
        .await
        .expect("the second flush returns at once");
    assert_eq!(ingest.inserted(), 1);
    ingest.unblock();
    gate.add_permits(1);
    settle(&logger).await;
}

#[tokio::test]
async fn zerobus_flush_state_waits_for_a_running_flush() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let gate = ingest.block();
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let logger = logger(params(1, 3600, false), client);
    logger
        .async_log_success_event(event("a"))
        .await
        .expect("the event is accepted");
    assert!(
        until(|| ingest.inserted() == 1).await,
        "the delivery starts"
    );
    logger
        .async_log_success_event(event("b"))
        .await
        .expect("the event is accepted");
    let inner = Arc::clone(&logger.inner);
    let tracker = TaskTracker::new();
    let second = tracker.spawn(async move { inner.flush_queue(false).await });
    sleep(Duration::from_millis(10)).await;
    ingest.unblock();
    gate.add_permits(1);
    second.await.expect("the second flush finishes");
    assert!(
        until(|| ingest.inserted() == 2).await,
        "the second flush sends the row that arrived during the first"
    );
    assert!(queued_ids(&logger).is_empty());
}

#[tokio::test]
async fn zerobus_flush_state_clears_the_in_flight_count() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let logger = logger(params(2, 3600, false), client);
    logger
        .async_log_success_event(event("a"))
        .await
        .expect("the event is accepted");
    logger
        .async_log_success_event(event("b"))
        .await
        .expect("the event is accepted");
    settle(&logger).await;
    assert_eq!(ingest.inserted(), 1);
    assert_eq!(logger.inner.inflight.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn zerobus_flush_state_requeues_rows_after_an_aborted_flush() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let _gate = ingest.block();
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let logger = logger(params(1, 3600, false), client);
    assert_eq!(push(&logger, "a"), 1);
    let inner = Arc::clone(&logger.inner);
    let aborted = timeout(Duration::from_millis(100), inner.flush_queue(false)).await;
    assert!(
        aborted.is_err(),
        "the flush finished before the gate opened"
    );
    assert!(!inner.flushing.load(Ordering::SeqCst));
    assert_eq!(ingest.inserted(), 1);
    assert_eq!(queued_ids(&logger), expected_ids(&["a"]));
}
